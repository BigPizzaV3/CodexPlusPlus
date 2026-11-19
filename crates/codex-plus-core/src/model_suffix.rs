//! model_list 后缀语法解析与 catalog JSON 构建。
//!
//! 后缀语法：`deepseek-v4-pro[1M]` 表示 slug=deepseek-v4-pro、context_window=1000000。
//! 单位 K/k=1000、M/m=1000000；纯数字也接受。后缀在生成 catalog 时剥离。

use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub slug: String,
    pub display_name: String,
    /// 来自后缀的窗口值；None 表示该条目无后缀（回落顶层默认）。
    pub suffix_window: Option<u64>,
    /// 显式自动压缩百分比，以百万分之一百分比为单位（90% = 90_000_000）。
    /// None 表示不覆盖 Codex 默认的自动压缩行为。
    pub auto_compact_percent: Option<u32>,
}

/// 剥掉合法窗口后缀，保留模型 ID 的大小写。
pub(crate) fn normalize_model_key(raw: &str) -> String {
    parse_model_suffix(raw).0
}

/// 优先精确匹配，再剥后缀，最后按大小写不敏感匹配历史键。
/// BTreeMap 保证同名大小写变体的回退结果稳定。
pub(crate) fn lookup_model_map<'a, T>(
    map: &'a std::collections::BTreeMap<String, T>,
    model: &str,
) -> Option<&'a T> {
    if let Some(value) = map.get(model) {
        return Some(value);
    }
    let slug = normalize_model_key(model);
    if let Some(value) = map.get(&slug) {
        return Some(value);
    }
    map.iter()
        .find(|(key, _)| normalize_model_key(key).eq_ignore_ascii_case(&slug))
        .map(|(_, value)| value)
}

/// 解析单个模型条目的后缀，返回 (slug, 可选窗口)。
/// 括号内非合法窗口 token 时，整串作为 slug 且 window=None（不剥离括号）。
pub fn parse_model_suffix(raw: &str) -> (String, Option<u64>) {
    let raw = raw.trim();
    // 仅当 ] 是最后一个字符时才视为后缀
    if let Some(close) = raw.rfind(']')
        && close == raw.len() - 1
        && let Some(open) = raw[..close].rfind('[')
    {
        let inner = raw[open + 1..close].trim();
        let slug = raw[..open].trim();
        if let Some(window) = parse_window_token(inner).filter(|_| !slug.is_empty()) {
            return (slug.to_string(), Some(window));
        }
    }
    (raw.to_string(), None)
}

/// 一次性迁移：把旧格式 `slug[suffix]` 的 model_list 拆成无后缀列表和窗口 map。
pub fn migrate_model_list_with_suffixes(model_list: &str) -> (String, HashMap<String, String>) {
    let mut clean_lines = Vec::new();
    let mut windows = HashMap::new();
    for raw in model_list
        .split(['\r', '\n', ','])
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        let (slug, window) = parse_model_suffix(raw);
        clean_lines.push(slug.clone());
        if let Some(window) = window {
            windows.insert(slug, window.to_string());
        }
    }
    (clean_lines.join("\n"), windows)
}

/// 解析括号内的窗口 token，如 "1M" / "200K" / "1000000"。非法或 0 返回 None。
pub(crate) fn parse_window_token(token: &str) -> Option<u64> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    let (num_part, multiplier) = match token.chars().last() {
        Some('K' | 'k') => (&token[..token.len() - 1], 1_000u64),
        Some('M' | 'm') => (&token[..token.len() - 1], 1_000_000u64),
        Some(_) => (token, 1u64),
        None => return None,
    };
    num_part
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|value| value.checked_mul(multiplier))
        .filter(|value| *value > 0)
}

/// 解析自动压缩百分比 token，如 "90"、"84.329412%"。
/// 返回百万分之一百分比，供 Rust 与前端使用同一套精度和舍入规则。
pub(crate) fn parse_compact_percent(token: &str) -> Option<u32> {
    let token = token.trim();
    let token = token.strip_suffix('%').unwrap_or(token).trim();
    if token.ends_with('%') {
        return None;
    }
    let (whole, fraction) = token.split_once('.').unwrap_or((token, ""));
    if fraction.len() > 6 || whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !fraction.is_empty() && !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole = whole.parse::<u32>().ok()?;
    let mut fraction_value = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u32>().ok()?
    };
    for _ in fraction.len()..6 {
        fraction_value = fraction_value.checked_mul(10)?;
    }
    let scaled = whole.checked_mul(1_000_000)?.checked_add(fraction_value)?;
    (scaled > 0 && scaled <= 100_000_000).then_some(scaled)
}

/// 收集 profile 的全部模型条目（当前 model + model_list），去重并从 `model_windows` map 读取窗口。
/// 返回顺序：当前 model 在前。用于生成 catalog，包含全部模型以避免
/// #1064 单模型副作用（catalog 只剩当前 model）。
///
/// 当前 model 若不带后缀，但在 `model_windows` 中存在同名条目，
/// 则采纳该窗口（让当前 model 的窗口也能生效）。
pub fn collect_catalog_entries(
    model_list: &str,
    model_windows: &HashMap<String, String>,
    model_auto_compact: &HashMap<String, String>,
    current_model: &str,
) -> Vec<ModelCatalogEntry> {
    // 先解析 model_list，保留顺序并去重；后缀已从 model_list 剥离，窗口来自 model_windows map。
    let windows = model_windows.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
    let auto_compacts = model_auto_compact.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
    let mut seen = HashSet::new();
    let mut list_entries: Vec<ModelCatalogEntry> = Vec::new();
    for raw in model_list
        .split(['\r', '\n', ','])
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let (slug, suffix_window) = parse_model_suffix(raw);
        if slug.is_empty() {
            continue;
        }
        if !seen.insert(slug.clone()) {
            continue;
        }
        let suffix_window = suffix_window.or_else(|| {
            lookup_model_map(&windows, &slug)
                .and_then(|token| parse_window_token(token))
        });
        let auto_compact_percent = lookup_model_map(&auto_compacts, &slug)
            .and_then(|token| parse_compact_percent(token));
        list_entries.push(ModelCatalogEntry {
            display_name: slug.clone(),
            slug,
            suffix_window,
            auto_compact_percent,
        });
    }

    // 处理当前 model，放到最前面。
    let current_model = current_model.trim();
    let mut entries = Vec::new();
    if !current_model.is_empty() {
        let (slug, suffix_window) = parse_model_suffix(current_model);
        if !slug.is_empty() {
            let suffix_window = suffix_window.or_else(|| {
                lookup_model_map(&windows, &slug)
                    .and_then(|token| parse_window_token(token))
            });
            let auto_compact_percent = lookup_model_map(&auto_compacts, &slug)
                .and_then(|token| parse_compact_percent(token));
            entries.push(ModelCatalogEntry {
                display_name: slug.clone(),
                slug: slug.clone(),
                suffix_window,
                auto_compact_percent,
            });
            // 从 list_entries 中移除同 slug 条目，避免重复。
            list_entries.retain(|entry| entry.slug != slug);
        }
    }

    entries.append(&mut list_entries);
    entries
}

/// 内置 codex bundled catalog 模板（assets/codex-models.json），用于 clone entry
/// 保证字段齐全，避免 codex 因缺字段忽略条目。
const BUNDLED_TEMPLATE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/codex-models.json"
));

const GPT56_METADATA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/gpt56-model-metadata-compat.json"
));

const DEEPSEEK_METADATA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/deepseek-model-metadata.json"
));

const ASTRA_METADATA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/astra-model-metadata-compat.json"
));

const GPT6_SOL_LUNA_METADATA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/gpt6-sol-luna-model-metadata-compat.json"
));
const GPT61_SOL_METADATA_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/gpt61-sol-model-metadata-compat.json"
));

/// 精调/供应商 metadata 来源。数组顺序就是覆盖优先级，条目和来源名始终成对维护。
const COMPATIBILITY_METADATA_SOURCES: &[(&'static str, &'static str)] = &[
    (GPT56_METADATA_JSON, "gpt-5.6 兼容"),
    (ASTRA_METADATA_JSON, "gpt-6-astra 兼容"),
    (DEEPSEEK_METADATA_JSON, "DeepSeek"),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/doubao-model-metadata.json"
        )),
        "豆包",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/gemini-model-metadata.json"
        )),
        "Gemini",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/glm-model-metadata.json"
        )),
        "GLM",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/grok-model-metadata.json"
        )),
        "Grok",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/kimi-model-metadata.json"
        )),
        "Kimi",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/mimo-model-metadata.json"
        )),
        "MiMo",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/minimax-model-metadata.json"
        )),
        "MiniMax",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/mistral-model-metadata.json"
        )),
        "Mistral",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/muse-model-metadata.json"
        )),
        "Muse",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/nvidia-model-metadata.json"
        )),
        "NVIDIA",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/qwen-model-metadata.json"
        )),
        "Qwen",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/stepfun-model-metadata.json"
        )),
        "StepFun",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/thinkingmachines-model-metadata.json"
        )),
        "Thinking Machines",
    ),
    (
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/gptoss-model-metadata.json"
        )),
        "gpt-oss",
    ),
    (GPT6_SOL_LUNA_METADATA_JSON, "gpt-6 Sol/Luna 兼容"),
    (GPT61_SOL_METADATA_JSON, "gpt-6.1 Sol 兼容"),
];

/// 该 slug 是否需要落一份内置元数据 catalog（无用户窗口/元数据时也要生成）。
/// 判定与生成链的模板查找保持一致：精调/供应商层、运行时官方缓存、bundled
/// 静态资产任一命中即算。
pub fn requires_bundled_metadata_catalog(slug: &str) -> bool {
    resolve_builtin_metadata(slug).is_some()
}

pub fn model_ui_metadata(slug: &str) -> Option<Value> {
    model_ui_metadata_with_override(slug, None)
}

/// UI 与请求拦截读取同一份供应商模型能力；用户明确声明优先于内置默认。
pub fn model_ui_metadata_with_override(slug: &str, user_override: Option<&Value>) -> Option<Value> {
    let resolved = resolve_builtin_metadata(slug);
    let has_builtin = resolved.is_some();
    let user_override = user_override.and_then(Value::as_object);
    if resolved.is_none() && user_override.is_none() {
        return None;
    }
    let normalized_slug = resolved.as_ref().map(|entry| entry.slug.clone())
        .unwrap_or_else(|| normalized_model_slug(slug));
    let mut metadata = resolved.map(|entry| entry.entry).unwrap_or_else(|| json!({}));
    if let Some(user) = user_override
        && let Some(model) = metadata.as_object_mut()
    {
        for (key, value) in user {
            if key != "codex_plus_fast_support" {
                model.insert(key.clone(), value.clone());
            }
        }
    }
    let fast_mode = user_override.and_then(|user| user.get("codex_plus_fast_support"));
    apply_model_fast_support(&mut metadata, fast_mode);
    let levels = metadata
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|level| {
            let effort = level.get("effort")?.as_str()?.trim();
            if effort.is_empty() {
                return None;
            }
            Some(json!({
                "reasoningEffort": effort,
                "description": level
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            }))
        })
        .collect::<Vec<_>>();
    let mut ui = json!({
        "displayName": metadata
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(normalized_slug.as_str()),
        "description": metadata
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("Custom model"),
        "defaultReasoningEffort": metadata
            .get("default_reasoning_level")
            .and_then(Value::as_str)
            .unwrap_or("medium"),
        "supportedReasoningEfforts": levels,
        "additionalSpeedTiers": metadata
            .get("additional_speed_tiers")
            .cloned()
            .unwrap_or_else(|| json!([])),
        "serviceTiers": metadata
            .get("service_tiers")
            .cloned()
            .unwrap_or_else(|| json!([]))
    });
    // 供应商分组身份（模型聚合在 catalog 条目上写的 `codex_plus_group` /
    // `codex_plus_group_id`）：这份 UI 元数据是白名单结构，不在这里显式转发就流不到
    // 前端。只在值是非空字符串时写——单供应商直连的目录没有这两个键，因此不会凭空
    // 多出一个空分组；空串同样忽略，避免前端渲染出无名表头。
    for key in ["codex_plus_group", "codex_plus_group_id"] {
        if let Some(value) = metadata
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            ui[key] = json!(value);
        }
    }
    // `codex_plus_default` 标的是"这个模型属于默认成员"，用于在选择器里给默认供应商
    // 那一组打标志。只有显式 true 才转发，缺键/false 都不写，保持既有响应形态不变。
    if metadata.get("codex_plus_default").and_then(Value::as_bool) == Some(true) {
        ui["codex_plus_default"] = json!(true);
    }
    // 网关写的收藏标记：选择器里在这一行后面画星标。同样只有显式 true 才转发。
    if metadata.get("codex_plus_favorite").and_then(Value::as_bool) == Some(true) {
        ui["codex_plus_favorite"] = json!(true);
    }
    // 注入侧与原生目录使用同一份窗口信息。
    for (source, camel, snake) in [
        ("context_window", "contextWindow", "context_window"),
        ("max_context_window", "maxContextWindow", "max_context_window"),
    ] {
        if let Some(window) = metadata.get(source) {
            ui[camel] = window.clone();
            ui[snake] = window.clone();
        }
    }
    // 图片输入能力（#3042）：原生描述符的 inputModalities 决定输入框能不能贴图。
    // 白名单不转发就流不到注入层，注入的描述符只能瞎猜。用户声明优先，其次内置/目录条目；
    // 两种写法都认，输出也同时带两种，与窗口字段一致。没声明就不写，交给注入层按原生处理。
    // 先看用户声明（两种写法），再看合并后的条目，避免用户的 camelCase 被内置的 snake_case 压住。
    let declared = |snake: &str, camel: &str, valid: fn(&Value) -> bool| {
        [user_override, metadata.as_object()]
            .into_iter()
            .flatten()
            .flat_map(|source| [source.get(snake), source.get(camel)])
            .flatten()
            .find(|value| valid(value))
            .cloned()
    };
    if let Some(modalities) = declared("input_modalities", "inputModalities", Value::is_array)
        .as_ref()
        .and_then(Value::as_array)
    {
        let modalities: Vec<Value> = modalities
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|modality| !modality.is_empty())
            .map(|modality| json!(modality))
            .collect();
        ui["inputModalities"] = json!(modalities);
        ui["input_modalities"] = json!(modalities);
    }
    if let Some(original) = declared(
        "supports_image_detail_original",
        "supportsImageDetailOriginal",
        Value::is_boolean,
    )
    .as_ref()
    .and_then(Value::as_bool)
    {
        ui["supportsImageDetailOriginal"] = json!(original);
        ui["supports_image_detail_original"] = json!(original);
    }
    if !has_builtin {
        // 单独声明 Fast 不能替原生描述符制造名称、描述或推理默认档。
        for (ui_field, source_field) in [
            ("displayName", "display_name"),
            ("description", "description"),
            ("defaultReasoningEffort", "default_reasoning_level"),
        ] {
            if metadata.get(source_field).and_then(Value::as_str).is_none() {
                ui.as_object_mut().unwrap().remove(ui_field);
            }
        }
    }
    // 内置空数组不代表用户禁用；只有用户 JSON 中明确的数组才覆盖硬名单。
    // 此标记仅属于 /model-catalog 的 UI 响应，不写入原生 models.json。
    if let Some(user) = user_override {
        let declaration = match fast_mode.and_then(Value::as_str) {
            Some("supported") => Some(true),
            Some("unsupported") => Some(false),
            _ => user.get("service_tiers").and_then(Value::as_array)
                .map(|tiers| tiers.iter().any(|tier| model_speed_tier_is(tier, "priority")))
                .or_else(|| user.get("additional_speed_tiers").and_then(Value::as_array)
                    .map(|tiers| tiers.iter().any(|tier| model_speed_tier_is(tier, "fast")))),
        };
        if let Some(supported) = declaration {
            ui["prioritySupportOverride"] = json!(supported);
        }
    }
    Some(ui)
}

pub(crate) fn model_speed_tier_is(tier: &Value, expected: &str) -> bool {
    tier.as_str().or_else(|| tier.get("id").and_then(Value::as_str))
        .is_some_and(|id| id.trim().eq_ignore_ascii_case(expected))
}

/// 生成原生 catalog 和 UI 元数据共用解释；私有声明不进入 Codex 的模型 schema。
pub(crate) fn apply_model_fast_support(model: &mut Value, declaration: Option<&Value>) {
    let supported = match declaration.and_then(Value::as_str) {
        Some("supported") => true,
        Some("unsupported") => false,
        _ => return,
    };
    for (field, tier, added) in [
        ("service_tiers", "priority", json!({"id":"priority", "name":"Fast", "description":"Priority processing declared for this provider."})),
        ("additional_speed_tiers", "fast", json!("fast")),
    ] {
        let mut values = model.get(field).and_then(Value::as_array).cloned().unwrap_or_default();
        if supported {
            if field == "service_tiers" {
                for value in values.iter_mut().filter(|value| model_speed_tier_is(value, tier)) {
                    let mut entry = value.as_object().cloned().unwrap_or_default();
                    for key in ["id", "name", "description"] {
                        if entry.get(key).is_none_or(|value| !value.is_string()) {
                            entry.insert(key.to_string(), added[key].clone());
                        }
                    }
                    *value = Value::Object(entry);
                }
            }
            if !values.iter().any(|value| model_speed_tier_is(value, tier)) { values.push(added); }
        } else {
            values.retain(|value| !model_speed_tier_is(value, tier));
        }
        model[field] = json!(values);
    }
}

/// 内置元数据匹配结果：来源名 + 完整条目（含窗口/展示/档位等字段）。
/// 供管理器「元数据导入区」显示匹配状态与预填文本使用。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuiltinModelMetadata {
    pub source: String,
    pub entry: Value,
}

#[derive(Debug, Clone)]
struct ResolvedBuiltinMetadata {
    slug: String,
    source: &'static str,
    entry: Value,
}

fn normalized_model_slug(slug: &str) -> String {
    parse_model_suffix(slug).0.trim().to_string()
}

fn resolve_compatibility_metadata(slug: &str) -> Option<ResolvedBuiltinMetadata> {
    COMPATIBILITY_METADATA_SOURCES
        .iter()
        .find_map(|(catalog_json, source)| {
            catalog_metadata_entry(catalog_json, slug).map(|entry| ResolvedBuiltinMetadata {
                slug: slug.to_string(),
                source,
                entry,
            })
        })
}

/// 按生成链优先级解析模型元数据：兼容层 → 运行时缓存 → bundled。
fn resolve_builtin_metadata(slug: &str) -> Option<ResolvedBuiltinMetadata> {
    let slug = normalized_model_slug(slug);
    if slug.is_empty() {
        return None;
    }
    if let Some(mut metadata) = resolve_compatibility_metadata(&slug) {
        // Compatibility metadata is an overlay. Keep the same runtime/bundled
        // base that the catalog builder uses, then apply its product/vendor
        // fields on top. This keeps UI lookup and generated catalogs identical.
        let mut base = runtime_models_cache_entry(&slug)
            .or_else(|| bundled_template_entry(&slug))
            .unwrap_or_else(|| fallback_template_entry(&slug));
        if let (Some(target), Some(source)) = (base.as_object_mut(), metadata.entry.as_object()) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
        metadata.entry = base;
        return Some(metadata);
    }
    if let Some(entry) = runtime_models_cache_entry(&slug) {
        return Some(ResolvedBuiltinMetadata {
            slug,
            source: "官方内置",
            entry,
        });
    }
    bundled_template_entry(&slug).map(|entry| ResolvedBuiltinMetadata {
        slug,
        source: "官方内置",
        entry,
    })
}

/// 按模型名查内置元数据（剥合法 suffix、大小写不敏感）。
pub fn builtin_model_metadata(slug: &str) -> Option<BuiltinModelMetadata> {
    resolve_builtin_metadata(slug).map(|metadata| BuiltinModelMetadata {
        source: metadata.source.to_string(),
        entry: metadata.entry,
    })
}

/// 内置元数据索引（管理器模型列表行级标记用）：嵌入层 + 运行时官方缓存全量。
pub fn builtin_model_metadata_index() -> Vec<Value> {
    // First gather candidate slugs, then resolve each through the same owned
    // resolver used by single-model lookup and catalog generation.
    let mut candidate_slugs = Vec::new();
    let mut candidate_seen = HashSet::new();
    fn collect_candidates(
        models: &[Value],
        candidate_seen: &mut HashSet<String>,
        candidate_slugs: &mut Vec<String>,
    ) {
        for entry in models {
            let Some(slug) = entry.get("slug").and_then(Value::as_str) else {
                continue;
            };
            let normalized = normalized_model_slug(slug);
            if !normalized.is_empty() && candidate_seen.insert(normalized.to_ascii_lowercase()) {
                candidate_slugs.push(normalized);
            }
        }
    }
    for (catalog_json, _) in COMPATIBILITY_METADATA_SOURCES {
        with_catalog_metadata_models(catalog_json, |models| {
            collect_candidates(models, &mut candidate_seen, &mut candidate_slugs);
        });
    }
    if let Some(models) = runtime_models_catalog() {
        collect_candidates(&models, &mut candidate_seen, &mut candidate_slugs);
    }
    collect_candidates(bundled_catalog_models(), &mut candidate_seen, &mut candidate_slugs);

    let mut seen = HashSet::new();
    let mut index = Vec::new();
    let mut push_resolved = |metadata: ResolvedBuiltinMetadata| {
        let entry = metadata.entry;
        let Some(slug) = entry.get("slug").and_then(Value::as_str) else {
            return;
        };
        if !seen.insert(slug.to_ascii_lowercase()) {
            return;
        }
        index.push(json!({
            "slug": slug,
            "source": metadata.source,
            "display_name": entry
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or(slug),
            "context_window": entry
                .get("context_window")
                .or_else(|| entry.get("max_context_window")),
            "auto_compact_token_limit": entry.get("auto_compact_token_limit"),
            // 模型列表「思考等级」列的初值：内置声明直接展示（用户覆盖写进
            // 自己的 model_metadata，不经这里）。
            "supported_reasoning_levels": entry.get("supported_reasoning_levels"),
            "default_reasoning_level": entry.get("default_reasoning_level"),
        }));
    };
    for slug in candidate_slugs {
        if let Some(metadata) = resolve_builtin_metadata(&slug) {
            push_resolved(metadata);
        }
    }
    index
}
/// 构建 codex model_catalog_json 内容。
///
/// 采用 template-clone 思路：取 codex 自带 bundled entry 做模板，
/// 再覆盖 slug / display_name / description / context_window / max_context_window /
/// effective_context_window_percent / priority / auto_compact_token_limit 等字段。
/// 无后缀条目用 fallback_window；fallback 也无时回落 272000（codex 默认）。
/// auto_compact_token_limit 仅在条目带显式百分比时写入；否则留 null，保持 Codex 默认行为。
pub fn build_model_catalog_json(
    entries: &[ModelCatalogEntry],
    fallback_window: Option<u64>,
) -> String {
    build_model_catalog_json_with_capabilities(entries, fallback_window, None, None, false, None)
}

/// 使用指定模板（或内置 bundled 模板）构建 catalog。
/// `template` 为单个 model entry 的 JSON Value；为 None 时使用内置模板的第一条。
pub fn build_model_catalog_json_with_template(
    entries: &[ModelCatalogEntry],
    fallback_window: Option<u64>,
    template: Option<&Value>,
) -> String {
    build_model_catalog_json_with_capabilities(entries, fallback_window, template, None, false, None)
}

/// 使用显式 provider capability 构建 catalog。
/// `use_responses_lite_override` 仅由明确知道 provider wire capability 的调用方传入；
/// 通用 builder 默认保留模板中的原始 Lite 行为。
/// `provider_multi_agent`（#2683）：供应商级 Multi-Agent 协议显式选择（"v1"/"v2"）。
/// 优先于 Lite 场景的自动 v2 与内置模板的每模型默认（gpt-5.6-sol→v2 等）——
/// 供应商的链路形状（能否承载 v2 加密载荷）对它名下所有模型一致成立；每模型
/// 用户元数据（modelMetadata 的 multi_agent_version）在后续
/// `apply_model_metadata_overrides` 里覆盖，保持「模型级 > 供应商级 > Auto」。
pub(crate) fn build_model_catalog_json_with_capabilities(
    entries: &[ModelCatalogEntry],
    fallback_window: Option<u64>,
    template: Option<&Value>,
    use_responses_lite_override: Option<bool>,
    deepseek_metadata: bool,
    provider_multi_agent: Option<&str>,
) -> String {
    let models: Vec<Value> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let (mut model, has_model_metadata) = if deepseek_metadata {
                deepseek_model_template_entry(&entry.slug)
                    .unwrap_or_else(|| model_template_entry(&entry.slug))
            } else {
                template
                    .cloned()
                    .map(|template| (template, false))
                    .unwrap_or_else(|| model_template_entry(&entry.slug))
            };
            let metadata_window = model.get("context_window").and_then(Value::as_u64);
            let context_window = entry
                .suffix_window
                .or(fallback_window)
                .or(metadata_window)
                .unwrap_or(272_000);
            model["slug"] = json!(entry.slug);
            // 借用模板不能证明自定义模型支持并行工具调用；精确元数据才有此权威。
            if !has_model_metadata || !model.get("supports_parallel_tool_calls").is_some_and(Value::is_boolean) {
                model["supports_parallel_tool_calls"] = json!(false);
            }
            if !has_model_metadata {
                model["display_name"] = json!(entry.display_name);
                model["description"] = json!(entry.display_name);
            }
            model["context_window"] = json!(context_window);
            // 受管目录的窗口可在会话冷恢复时覆盖；固定 max 会把后续调大的值截回启动时窗口。
            model["max_context_window"] = Value::Null;
            // 通用自定义模型显示完整窗口；DeepSeek Responses 保留官方目录的 95%。
            if !deepseek_metadata {
                model["effective_context_window_percent"] = json!(100);
            }
            if let Some(compact_percent) = entry.auto_compact_percent {
                let compact_limit = ((context_window as u128 * compact_percent as u128
                    + 50_000_000)
                    / 100_000_000) as u64;
                model["auto_compact_token_limit"] = json!(compact_limit.max(1));
            } else {
                model["auto_compact_token_limit"] = Value::Null;
            }
            model["priority"] = json!(1000 + index);
            model["visibility"] =
                json!(crate::aggregate_catalog::catalog_visibility(&entry.slug));
            // Custom Responses relay catalogs must advertise the v2 multi-agent
            // contract so Codex exposes the sub-agent tools.
            // 供应商级显式选择（#2683）优先：链路能否承载 v2 加密载荷对整家供应商
            // 一致成立，因此同时压过 Lite 场景的自动 v2 与内置模板的每模型默认。
            if let Some(version) = provider_multi_agent {
                model["multi_agent_version"] = json!(version);
            } else if use_responses_lite_override.is_some() {
                model["multi_agent_version"] = json!("v2");
            }
            if !deepseek_metadata {
                model["supported_in_api"] = json!(true);
            }
            if let Some(use_responses_lite) = use_responses_lite_override {
                model["use_responses_lite"] = json!(use_responses_lite);
            }
            if !has_model_metadata {
                model["additional_speed_tiers"] = json!([]);
                model["service_tiers"] = json!([]);
            }
            // Codex Beta 26.727 起每个 model entry 都必须带 base_instructions，
            // 否则整份 model_catalog_json 解析失败并连带 config.toml 拒载（#2492）。
            // bundled 模板首条（gpt-5.5）没有该字段，clone 后按 Codex 自身的
            // legacy 序列化规则补齐：镜像条目里的 instructions_template。
            if model.get("base_instructions").is_none_or(Value::is_null) {
                let mirrored = model
                    .get("model_messages")
                    .and_then(|messages| messages.get("instructions_template"))
                    .filter(|value| value.is_string())
                    .cloned()
                    .unwrap_or_else(|| json!(""));
                model["base_instructions"] = mirrored;
            }
            model["availability_nux"] = Value::Null;
            model["upgrade"] = Value::Null;
            model
        })
        .collect();
    serde_json::to_string_pretty(&json!({ "models": models })).unwrap_or_default()
}

fn deepseek_model_template_entry(slug: &str) -> Option<(Value, bool)> {
    let compatibility = catalog_metadata_entry(DEEPSEEK_METADATA_JSON, slug)?;
    let mut template = first_bundled_template_entry().unwrap_or_else(|| json!({}));
    if let (Some(target), Some(source)) = (template.as_object_mut(), compatibility.as_object()) {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    Some((template, true))
}

fn model_template_entry(slug: &str) -> (Value, bool) {
    if let Some(metadata) = resolve_builtin_metadata(slug) {
        return (metadata.entry, true);
    }
    // 同家族模板只是回退底板：has_model_metadata 仍为 false，展示名/描述/并行工具调用
    // 等字段继续按未命中处理，只继承家族的身份文案与能力字段。
    (fallback_template_entry(slug), false)
}

/// 本机官方缓存（`models_cache.json`）给这个模型的上下文窗口，即登录账号实际拿到的窗口。
/// 不经过兼容层：兼容层的窗口是给第三方中转的，官方账号以原生资料为准（#3161）。
pub fn native_cache_context_window(slug: &str) -> Option<u64> {
    runtime_models_cache_entry(&normalized_model_slug(slug))?
        .get("context_window")?
        .as_u64()
        .filter(|window| *window > 0)
}

/// 从用户本机 codex 官方缓存读取同 slug 条目。
/// 缓存由官方 App 登录态维护，这里只读不写；文件缺失或解析失败时静默回落静态资产。
fn runtime_models_cache_entry(slug: &str) -> Option<Value> {
    with_runtime_models(|models| find_catalog_entry(models, slug).cloned()).flatten()
}

#[derive(Clone)]
struct RuntimeCatalogCacheEntry {
    modified: Option<SystemTime>,
    length: u64,
    models: Vec<Value>,
}

/// 缓存运行时 models_cache.json 的解析结果；以路径和文件指纹隔离 CODEX_HOME，
/// 文件变更后重新解析，避免索引对每个候选 slug 重复读盘和反序列化。
fn with_runtime_models<T>(f: impl FnOnce(&[Value]) -> T) -> Option<T> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, RuntimeCatalogCacheEntry>>> = OnceLock::new();
    let path = crate::codex_home::default_codex_home_dir().join("models_cache.json");
    let metadata = std::fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok();
    let length = metadata.len();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = cache.get(&path)
        && cached.modified == modified
        && cached.length == length
    {
        return Some(f(&cached.models));
    }
    let contents = std::fs::read_to_string(&path).ok()?;
    let catalog: Value = serde_json::from_str(&contents).ok()?;
    let models = catalog.get("models")?.as_array()?.clone();
    cache.insert(
        path,
        RuntimeCatalogCacheEntry {
            modified,
            length,
            models: models.clone(),
        },
    );
    Some(f(&models))
}

fn runtime_models_catalog() -> Option<Vec<Value>> {
    with_runtime_models(|models| models.to_vec())
}

/// 按 slug 查找 catalog 条目：先精确匹配，未命中再按大小写不敏感匹配。
fn find_catalog_entry<'a>(models: &'a [Value], slug: &str) -> Option<&'a Value> {
    models
        .iter()
        .find(|entry| entry.get("slug").and_then(Value::as_str) == Some(slug))
        .or_else(|| {
            models.iter().find(|entry| {
                entry
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(slug))
            })
        })
}

#[doc(hidden)]
pub fn find_catalog_entry_for_test<'a>(models: &'a [Value], slug: &str) -> Option<&'a Value> {
    find_catalog_entry(models, slug)
}

fn bundled_template_entry(slug: &str) -> Option<Value> {
    find_catalog_entry(bundled_catalog_models(), slug).cloned()
}

fn first_bundled_template_entry() -> Option<Value> {
    bundled_catalog_models().first().cloned()
}

/// `gpt-<主版本>[.<次版本>][-<系列>]` 形态的 slug 拆解结果，仅用于挑选回退模板。
#[derive(Debug, Clone, PartialEq, Eq)]
struct GptSlug {
    major: u32,
    minor: u32,
    /// 版本号之后的部分（astra / sol / codex-spark 等），无系列时为空串。
    series: String,
}

/// 解析 `gpt-6.2-astra` 这类 slug；`gpt-oss-*`、`gpt-reserve`、`gpt-4o` 等
/// 版本段不是纯数字的形态返回 None，让它们继续走原有的首条模板回退。
fn parse_gpt_slug(slug: &str) -> Option<GptSlug> {
    fn digits(text: &str) -> Option<u32> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        text.parse().ok()
    }
    let lowered = slug.trim().to_ascii_lowercase();
    let rest = lowered.strip_prefix("gpt-")?;
    let (version, series) = rest.split_once('-').unwrap_or((rest, ""));
    let (major, minor) = version.split_once('.').unwrap_or((version, "0"));
    Some(GptSlug {
        major: digits(major)?,
        minor: digits(minor)?,
        series: series.to_string(),
    })
}

/// 没有精确条目时，在运行时缓存 + bundled 里挑同家族的模板 slug。
///
/// 只考虑同主版本（gpt-6* 不借 gpt-5*，否则会克隆出 "based on GPT-5" 的身份文案，#3180）。
/// 同主版本内排序：同系列优先 → 已知的旧版本优先于更新的版本 → 版本距离近的优先；
/// 仍并列时按「运行时缓存在前、bundled 按目录顺序」取最先出现的。
fn family_template_slug(slug: &str) -> Option<String> {
    let target = parse_gpt_slug(slug)?;
    // 只修跨代的情况（#3180）：通用回退模板（bundled 首条）与目标同主版本时，身份文案本来就对，
    // 保持原样——否则 gpt-5、gpt-5.1-codex-max 这类未知 gpt-5.x 会改借更新的 gpt-5.2 等模板，
    // shell_type、截断策略和自我介绍都跟着变。
    let fallback_major = first_bundled_template_entry()
        .and_then(|entry| entry.get("slug")?.as_str().and_then(parse_gpt_slug))
        .map(|parsed| parsed.major);
    if fallback_major == Some(target.major) {
        return None;
    }
    fn slugs_of(models: &[Value]) -> Vec<String> {
        let slug_of = |entry: &Value| entry.get("slug")?.as_str().map(str::to_string);
        models.iter().filter_map(slug_of).collect()
    }
    let mut candidates = with_runtime_models(slugs_of).unwrap_or_default();
    candidates.extend(slugs_of(bundled_catalog_models()));
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let parsed = parse_gpt_slug(&candidate)?;
            (parsed.major == target.major).then_some((candidate, parsed))
        })
        .min_by_key(|(_, parsed)| {
            (
                parsed.series != target.series,
                parsed.minor > target.minor,
                parsed.minor.abs_diff(target.minor),
            )
        })
        .map(|(candidate, _)| candidate)
}

/// 同家族回退模板：取家族 slug 的运行时/bundled 条目，并叠加它自己的兼容层字段，
/// 与精确命中该家族 slug 时看到的能力字段一致。
/// 这里刻意不走 `resolve_builtin_metadata`，避免它的兼容层回退再回头调用本函数。
fn family_template_entry(slug: &str) -> Option<Value> {
    let family_slug = family_template_slug(slug)?;
    let mut entry = runtime_models_cache_entry(&family_slug)
        .or_else(|| bundled_template_entry(&family_slug))?;
    if let Some(compat) = resolve_compatibility_metadata(&family_slug)
        && let (Some(target), Some(source)) = (entry.as_object_mut(), compat.entry.as_object())
    {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
    Some(entry)
}

/// 精确元数据缺失时的 clone 底板：先找同家族模板，找不到再退回 bundled 首条，
/// 保证未知自定义模型照常能生成条目。
fn fallback_template_entry(slug: &str) -> Value {
    family_template_entry(slug)
        .or_else(first_bundled_template_entry)
        .unwrap_or_else(|| json!({}))
}

fn bundled_catalog_models() -> &'static Vec<Value> {
    static MODELS: OnceLock<Vec<Value>> = OnceLock::new();
    MODELS.get_or_init(|| {
        serde_json::from_str::<Value>(BUNDLED_TEMPLATE_JSON)
            .ok()
            .and_then(|catalog| catalog.get("models").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    })
}

/// 未命中内置元数据时生成链的真实回退，供管理器显示。
pub fn fallback_template_info() -> Option<(String, u64)> {
    let entry = first_bundled_template_entry()?;
    let slug = entry.get("slug")?.as_str()?.to_string();
    let context_window = entry
        .get("context_window")
        .and_then(Value::as_u64)
        .unwrap_or(272_000);
    Some((slug, context_window))
}

/// 缓存每份静态 catalog 的解析结果，但只在锁内完成查找并返回 owned Value。
fn catalog_metadata_entry(catalog_json: &'static str, slug: &str) -> Option<Value> {
    with_catalog_metadata_models(catalog_json, |models| find_catalog_entry(models, slug).cloned())
}

fn with_catalog_metadata_models<T>(
    catalog_json: &'static str,
    f: impl FnOnce(&[Value]) -> T,
) -> T {
    static INDEXES: OnceLock<Mutex<HashMap<&'static str, Vec<Value>>>> = OnceLock::new();
    let indexes = INDEXES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut indexes = indexes
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entries = indexes.entry(catalog_json).or_insert_with(|| {
        serde_json::from_str::<Value>(catalog_json)
            .ok()
            .and_then(|catalog| catalog.get("models").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    });
    f(entries)
}
