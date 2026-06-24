//! model_list 后缀语法解析与 catalog JSON 构建。
//!
//! 后缀语法：`deepseek-v4-pro[1M]` 表示 slug=deepseek-v4-pro、context_window=1000000。
//! 单位 K/k=1000、M/m=1000000；纯数字也接受。后缀在生成 catalog 时剥离。

use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub slug: String,
    pub display_name: String,
    /// 来自后缀的窗口值；None 表示该条目无后缀（回落顶层默认）。
    pub suffix_window: Option<u64>,
}

/// 解析单个模型条目的后缀，返回 (slug, 可选窗口)。
/// 括号内非合法窗口 token 时，整串作为 slug 且 window=None（不剥离括号）。
pub fn parse_model_suffix(raw: &str) -> (String, Option<u64>) {
    let raw = raw.trim();
    if let Some(close) = raw.rfind(']') {
        // 仅当 ] 是最后一个字符时才视为后缀
        if close == raw.len() - 1 {
            if let Some(open) = raw[..close].rfind('[') {
                let inner = raw[open + 1..close].trim();
                let slug = raw[..open].trim();
                if !slug.is_empty() {
                    if let Some(window) = parse_window_token(inner) {
                        return (slug.to_string(), Some(window));
                    }
                }
            }
        }
    }
    (raw.to_string(), None)
}

/// 解析括号内的窗口 token，如 "1M" / "200K" / "1000000"。非法或 0 返回 None。
fn parse_window_token(token: &str) -> Option<u64> {
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
        .map(|value| value * multiplier)
        .filter(|value| *value > 0)
}

/// 收集 profile 的全部模型条目（当前 model + model_list），去重并解析后缀。
/// 返回顺序：当前 model 在前。用于生成 catalog，包含全部模型以避免
/// #1064 单模型副作用（catalog 只剩当前 model）。
///
/// 当前 model 若不带后缀，但在 model_list 中存在同名且带后缀的条目，
/// 则采纳该后缀（让当前 model 的窗口也能生效）。
pub fn collect_catalog_entries(model_list: &str, current_model: &str) -> Vec<ModelCatalogEntry> {
    // 先解析 model_list，保留顺序并去重。
    let mut seen = HashSet::new();
    let mut list_entries = Vec::new();
    let mut suffix_for_slug: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for raw in model_list
        .split(['\r', '\n', ','])
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let (slug, suffix_window) = parse_model_suffix(raw);
        if slug.is_empty() || !seen.insert(slug.clone()) {
            continue;
        }
        if let Some(window) = suffix_window {
            suffix_for_slug.entry(slug.clone()).or_insert(window);
        }
        list_entries.push(ModelCatalogEntry {
            display_name: slug.clone(),
            slug,
            suffix_window,
        });
    }

    // 处理当前 model，放到最前面。
    let current_model = current_model.trim();
    let mut entries = Vec::new();
    if !current_model.is_empty() {
        let (slug, mut suffix_window) = parse_model_suffix(current_model);
        if !slug.is_empty() {
            if suffix_window.is_none() {
                if let Some(window) = suffix_for_slug.get(&slug) {
                    suffix_window = Some(*window);
                }
            }
            entries.push(ModelCatalogEntry {
                display_name: slug.clone(),
                slug: slug.clone(),
                suffix_window,
            });
            // 从 list_entries 中移除同 slug 条目，避免重复。
            list_entries.retain(|entry| entry.slug != slug);
        }
    }

    entries.append(&mut list_entries);
    entries
}

/// 构建 codex model_catalog_json 内容。条目字段对齐 cc-switch 覆盖集与 codex
/// 内置目录必要字段（见 docs/research/01-调研结果.md 第五节）。
/// 无后缀条目用 fallback_window；fallback 也无时回落 272000（codex 默认）。
/// auto_compact_token_limit 留 null：codex 内置模型即 null（按比例算，调研第六节）。
pub fn build_model_catalog_json(
    entries: &[ModelCatalogEntry],
    fallback_window: Option<u64>,
) -> String {
    let models: Vec<Value> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let context_window = entry
                .suffix_window
                .or(fallback_window)
                .unwrap_or(272_000);
            json!({
                "slug": entry.slug,
                "display_name": entry.display_name,
                "description": entry.display_name,
                "context_window": context_window,
                "max_context_window": context_window,
                "auto_compact_token_limit": Value::Null,
                "priority": 1000 + index,
                "visibility": "list",
                "supported_in_api": true,
                "additional_speed_tiers": [],
                "service_tiers": [],
                "availability_nux": Value::Null,
                "upgrade": Value::Null,
            })
        })
        .collect();
    serde_json::to_string_pretty(&json!({ "models": models })).unwrap_or_default()
}
