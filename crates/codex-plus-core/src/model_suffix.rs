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
            model_windows
                .get(&slug)
                .and_then(|token| parse_window_token(token))
        });
        let auto_compact_percent = model_auto_compact
            .get(&slug)
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
                model_windows
                    .get(&slug)
                    .and_then(|token| parse_window_token(token))
            });
            let auto_compact_percent = model_auto_compact
                .get(&slug)
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
];

/// 该 slug 是否需要落一份内置元数据 catalog（无用户窗口/元数据时也要生成）。
/// 判定与生成链的模板查找保持一致：精调/供应商层、运行时官方缓存、bundled
/// 静态资产任一命中即算。
pub fn requires_bundled_metadata_catalog(slug: &str) -> bool {
    resolve_builtin_metadata(slug).is_some()
}

pub fn model_ui_metadata(slug: &str) -> Option<Value> {
    let resolved = resolve_builtin_metadata(slug)?;
    let metadata = resolved.entry;
    let normalized_slug = resolved.slug;
    let levels = metadata
        .get("supported_reasoning_levels")?
        .as_array()?
        .iter()
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
    Some(json!({
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
    }))
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
            .unwrap_or_else(|| first_bundled_template_entry().unwrap_or_else(|| json!({})));
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
/// 采用 cc-switch 的 template-clone 思路：取 codex 自带 bundled entry 做模板，
/// 再覆盖 slug / display_name / description / context_window / max_context_window /
/// effective_context_window_percent / priority / auto_compact_token_limit 等字段。
/// 无后缀条目用 fallback_window；fallback 也无时回落 272000（codex 默认）。
/// auto_compact_token_limit 仅在条目带显式百分比时写入；否则留 null，保持 Codex 默认行为。
pub fn build_model_catalog_json(
    entries: &[ModelCatalogEntry],
    fallback_window: Option<u64>,
) -> String {
    build_model_catalog_json_with_capabilities(entries, fallback_window, None, None, false)
}

/// 使用指定模板（或内置 bundled 模板）构建 catalog。
/// `template` 为单个 model entry 的 JSON Value；为 None 时使用内置模板的第一条。
pub fn build_model_catalog_json_with_template(
    entries: &[ModelCatalogEntry],
    fallback_window: Option<u64>,
    template: Option<&Value>,
) -> String {
    build_model_catalog_json_with_capabilities(entries, fallback_window, template, None, false)
}

/// 使用显式 provider capability 构建 catalog。
/// `use_responses_lite_override` 仅由明确知道 provider wire capability 的调用方传入；
/// 通用 builder 默认保留模板中的原始 Lite 行为。
pub(crate) fn build_model_catalog_json_with_capabilities(
    entries: &[ModelCatalogEntry],
    fallback_window: Option<u64>,
    template: Option<&Value>,
    use_responses_lite_override: Option<bool>,
    deepseek_metadata: bool,
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
            let metadata_max_window = model.get("max_context_window").and_then(Value::as_u64);
            let context_window = entry
                .suffix_window
                .or(fallback_window)
                .or(metadata_window)
                .unwrap_or(272_000);
            // 用户显式配置窗口（后缀 / 每模型窗口 / profile 全局）时两字段同值；
            // 未显式配置时保留官方模板的 max_context_window 上限——gpt-5.6 / gpt-6
            // 官方为 272000/872000，压平成同值会把 codex 侧 872K 能力上限写低（#2191）。
            let max_context_window = entry
                .suffix_window
                .or(fallback_window)
                .unwrap_or_else(|| metadata_max_window.unwrap_or(context_window));
            model["slug"] = json!(entry.slug);
            if !has_model_metadata {
                model["display_name"] = json!(entry.display_name);
                model["description"] = json!(entry.display_name);
            }
            model["context_window"] = json!(context_window);
            model["max_context_window"] = json!(max_context_window);
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
            model["visibility"] = json!("list");
            // Custom Responses relay catalogs must advertise the v2 multi-agent
            // contract so Codex exposes the sub-agent tools.
            if use_responses_lite_override.is_some() {
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
    (
        first_bundled_template_entry().unwrap_or_else(|| json!({})),
        false,
    )
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
