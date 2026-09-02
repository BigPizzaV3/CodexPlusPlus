use std::collections::HashMap;

use codex_plus_core::model_suffix::{
    build_model_catalog_json, build_model_catalog_json_with_template, collect_catalog_entries,
    model_ui_metadata, parse_model_suffix,
};

#[test]
fn parse_suffix_extracts_k_and_m_units() {
    assert_eq!(
        parse_model_suffix("deepseek-v4-pro[1M]"),
        ("deepseek-v4-pro".to_string(), Some(1_000_000))
    );
    assert_eq!(
        parse_model_suffix("claude-sonnet-4[200K]"),
        ("claude-sonnet-4".to_string(), Some(200_000))
    );
    assert_eq!(
        parse_model_suffix("gpt-5.5[512k]"),
        ("gpt-5.5".to_string(), Some(512_000))
    );
    assert_eq!(
        parse_model_suffix("gpt-5.5[1000000]"),
        ("gpt-5.5".to_string(), Some(1_000_000))
    );
}

#[test]
fn parse_suffix_returns_none_without_bracket() {
    assert_eq!(parse_model_suffix("gpt-5.5"), ("gpt-5.5".to_string(), None));
    assert_eq!(
        parse_model_suffix("  qwen3-coder  "),
        ("qwen3-coder".to_string(), None)
    );
}

#[test]
fn parse_suffix_keeps_original_slug_when_bracket_invalid() {
    // 括号内非合法窗口 token 时，整串（含括号）作为 slug，window=None
    let (slug, window) = parse_model_suffix("foo[bar]");
    assert_eq!(slug, "foo[bar]");
    assert_eq!(window, None);

    // 括号未闭合：不剥离
    let (slug2, window2) = parse_model_suffix("foo[1M");
    assert_eq!(slug2, "foo[1M");
    assert_eq!(window2, None);
}

#[test]
fn parse_suffix_rejects_zero_and_negative() {
    assert_eq!(parse_model_suffix("foo[0K]"), ("foo[0K]".to_string(), None));
}

#[test]
fn collect_entries_includes_current_model_and_strips_suffix() {
    let mut windows = HashMap::new();
    windows.insert("deepseek-v4-pro".to_string(), "1M".to_string());
    let entries =
        collect_catalog_entries(
            "deepseek-v4-pro\nqwen3-coder",
            &windows,
            &HashMap::new(),
            "deepseek-v4-pro",
        );
    // 当前 model 与列表去重后共 2 条
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].slug, "deepseek-v4-pro");
    assert_eq!(entries[0].suffix_window, Some(1_000_000));
    assert_eq!(entries[1].slug, "qwen3-coder");
    assert_eq!(entries[1].suffix_window, None);
}

#[test]
fn collect_entries_deduplicates() {
    let entries =
        collect_catalog_entries(
            "qwen3-coder\nqwen3-coder",
            &HashMap::new(),
            &HashMap::new(),
            "qwen3-coder",
        );
    assert_eq!(entries.len(), 1);
}

#[test]
fn build_catalog_json_writes_context_window_and_strips_suffix() {
    let mut windows = HashMap::new();
    windows.insert("deepseek-v4-pro".to_string(), "1M".to_string());
    windows.insert("claude-sonnet-4".to_string(), "200K".to_string());
    let entries = collect_catalog_entries(
        "deepseek-v4-pro\nclaude-sonnet-4",
        &windows,
        &HashMap::new(),
        "",
    );
    let catalog = build_model_catalog_json(&entries, None);
    assert!(catalog.contains(r#""slug": "deepseek-v4-pro""#));
    assert!(catalog.contains(r#""context_window": 1000000"#));
    assert!(catalog.contains(r#""max_context_window": 1000000"#));
    assert!(catalog.contains(r#""slug": "claude-sonnet-4""#));
    assert!(catalog.contains(r#""context_window": 200000"#));
    // 后缀不得进入 catalog
    assert!(!catalog.contains("[1M]"));
    assert!(!catalog.contains("[200K]"));
    // auto_compact 留 null（codex 按比例算）
    assert!(catalog.contains(r#""auto_compact_token_limit": null"#));
}

#[test]
fn build_catalog_json_uses_fallback_for_no_suffix_entries() {
    let entries = collect_catalog_entries("qwen3-coder", &HashMap::new(), &HashMap::new(), "");
    let catalog = build_model_catalog_json(&entries, Some(272_000));
    assert!(catalog.contains(r#""slug": "qwen3-coder""#));
    assert!(catalog.contains(r#""context_window": 272000"#));
}

#[test]
fn build_catalog_json_uses_runtime_compatible_gpt56_metadata() {
    let entries = collect_catalog_entries(
        "gpt-5.6-sol\ngpt-5.6-terra\ngpt-5.6-luna",
        &HashMap::new(),
        &HashMap::new(),
        "gpt-5.6-sol",
    );
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    let models = catalog["models"].as_array().unwrap();

    for (slug, default_reasoning, expected_efforts) in [
        (
            "gpt-5.6-sol",
            "low",
            vec!["low", "medium", "high", "xhigh", "max", "ultra"],
        ),
        (
            "gpt-5.6-terra",
            "medium",
            vec!["low", "medium", "high", "xhigh", "max", "ultra"],
        ),
        (
            "gpt-5.6-luna",
            "medium",
            vec!["low", "medium", "high", "xhigh", "max"],
        ),
    ] {
        let model = models.iter().find(|model| model["slug"] == slug).unwrap();
        let efforts = model["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["effort"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(model["context_window"], 272_000);
        assert_eq!(model["max_context_window"], 272_000);
        assert_eq!(model["default_reasoning_level"], default_reasoning);
        assert_eq!(efforts, expected_efforts);
        assert!(!efforts.contains(&"minimal"));
        assert_eq!(model["additional_speed_tiers"], serde_json::json!(["fast"]));
        assert_eq!(model["service_tiers"][0]["id"], "priority");
        assert_eq!(model["supports_search_tool"], true);
        assert_eq!(model["use_responses_lite"], true);
    }
}

#[test]
fn build_catalog_json_preserves_template_responses_lite_behavior() {
    let entries = collect_catalog_entries(
        "official-model",
        &HashMap::new(),
        &HashMap::new(),
        "official-model",
    );
    let template = serde_json::json!({
        "slug": "official-template",
        "supports_search_tool": true,
        "use_responses_lite": true
    });
    let catalog: serde_json::Value = serde_json::from_str(&build_model_catalog_json_with_template(
        &entries,
        None,
        Some(&template),
    ))
    .unwrap();

    assert_eq!(catalog["models"][0]["use_responses_lite"], true);
    assert_eq!(catalog["models"][0]["supports_search_tool"], true);
}

#[test]
fn model_ui_metadata_exposes_fast_service_tier_capability() {
    let metadata = model_ui_metadata("gpt-5.6-sol").expect("Sol metadata should exist");

    assert_eq!(
        metadata["additionalSpeedTiers"],
        serde_json::json!(["fast"])
    );
    assert_eq!(metadata["serviceTiers"][0]["id"], "priority");
}

#[test]
fn collect_entries_adopts_suffix_for_current_model_from_list() {
    // 当前 model 本身无后缀，但 model_list 中靠后位置有同名带后缀条目。
    let mut windows = HashMap::new();
    windows.insert("deepseek-v4-pro".to_string(), "1M".to_string());
    let entries =
        collect_catalog_entries(
            "qwen3-coder\ndeepseek-v4-pro",
            &windows,
            &HashMap::new(),
            "deepseek-v4-pro",
        );
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].slug, "deepseek-v4-pro");
    assert_eq!(entries[0].suffix_window, Some(1_000_000));
}

#[test]
fn collect_entries_prefers_later_suffix_for_duplicate_slug() {
    // 同一 slug 先出现无后缀条目，后出现带后缀条目，应采纳后者窗口。
    let mut windows = HashMap::new();
    windows.insert("deepseek/deepseek-v4-flash".to_string(), "1M".to_string());
    let entries = collect_catalog_entries(
        "deepseek/deepseek-v4-flash\ndeepseek/deepseek-v4-flash",
        &windows,
        &HashMap::new(),
        "",
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].slug, "deepseek/deepseek-v4-flash");
    assert_eq!(entries[0].suffix_window, Some(1_000_000));
}

#[test]
fn collect_entries_prefers_later_suffix_when_reversed() {
    // 同一 slug 先出现 [1M]，后出现 [200K]，后者应覆盖前者。
    let mut windows = HashMap::new();
    windows.insert("deepseek/deepseek-v4-flash".to_string(), "200K".to_string());
    let entries = collect_catalog_entries(
        "deepseek/deepseek-v4-flash\ndeepseek/deepseek-v4-flash",
        &windows,
        &HashMap::new(),
        "",
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].slug, "deepseek/deepseek-v4-flash");
    assert_eq!(entries[0].suffix_window, Some(200_000));
}

#[test]
fn migrate_model_list_with_suffixes_splits_slug_and_window() {
    let input = "deepseek-v4-flash[1M]\ndeepseek-v4-pro\nnvidia/...:free[200K]";
    let (clean_list, windows) =
        codex_plus_core::model_suffix::migrate_model_list_with_suffixes(input);
    assert_eq!(
        clean_list,
        "deepseek-v4-flash\ndeepseek-v4-pro\nnvidia/...:free"
    );
    assert_eq!(
        windows.get("deepseek-v4-flash"),
        Some(&"1000000".to_string())
    );
    assert_eq!(windows.get("deepseek-v4-pro"), None);
    assert_eq!(windows.get("nvidia/...:free"), Some(&"200000".to_string()));
}

#[test]
fn build_catalog_json_writes_explicit_auto_compact_percent_only() {
    let mut windows = HashMap::new();
    windows.insert("deepseek-v4-pro".to_string(), "1M".to_string());
    let mut compacts = HashMap::new();
    compacts.insert("deepseek-v4-pro".to_string(), "80".to_string());
    let entries = collect_catalog_entries("deepseek-v4-pro", &windows, &compacts, "");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    assert_eq!(catalog["models"][0]["context_window"], 1_000_000);
    assert_eq!(catalog["models"][0]["auto_compact_token_limit"], 800_000);

    let default_entries = collect_catalog_entries(
        "default-model",
        &HashMap::new(),
        &HashMap::new(),
        "",
    );
    let default_catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&default_entries, Some(200_000))).unwrap();
    assert_eq!(default_catalog["models"][0]["auto_compact_token_limit"], serde_json::Value::Null);
}

#[test]
fn build_catalog_json_accepts_decimal_percent_and_rounds_half_up() {
    let mut windows = HashMap::new();
    windows.insert("gpt-5.6-sol".to_string(), "272000".to_string());
    let mut compacts = HashMap::new();
    compacts.insert("gpt-5.6-sol".to_string(), "84.329412%".to_string());
    let entries = collect_catalog_entries("gpt-5.6-sol", &windows, &compacts, "");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    assert_eq!(catalog["models"][0]["auto_compact_token_limit"], 229_376);

    let mut tiny_windows = HashMap::new();
    tiny_windows.insert("rounding-model".to_string(), "3".to_string());
    let mut tiny_compacts = HashMap::new();
    tiny_compacts.insert("rounding-model".to_string(), "50%".to_string());
    let tiny_entries = collect_catalog_entries("rounding-model", &tiny_windows, &tiny_compacts, "");
    let tiny_catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&tiny_entries, None)).unwrap();
    assert_eq!(tiny_catalog["models"][0]["auto_compact_token_limit"], 2);
}

#[test]
fn build_catalog_json_recalculates_threshold_when_window_changes() {
    let mut compacts = HashMap::new();
    compacts.insert("gpt-5.6-sol".to_string(), "84.329412%".to_string());
    let mut original_windows = HashMap::new();
    original_windows.insert("gpt-5.6-sol".to_string(), "272000".to_string());
    let original = collect_catalog_entries("gpt-5.6-sol", &original_windows, &compacts, "");
    let mut changed_windows = HashMap::new();
    changed_windows.insert("gpt-5.6-sol".to_string(), "800000".to_string());
    let changed = collect_catalog_entries("gpt-5.6-sol", &changed_windows, &compacts, "");
    assert_eq!(changed[0].auto_compact_percent, original[0].auto_compact_percent);
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&changed, None)).unwrap();
    assert_eq!(catalog["models"][0]["auto_compact_token_limit"], 674_635);
}

#[test]
fn build_catalog_json_ignores_invalid_percent_and_relay_validation_rejects_it() {
    let mut windows = HashMap::new();
    windows.insert("model".to_string(), "1M".to_string());
    let mut compacts = HashMap::new();
    compacts.insert("model".to_string(), "90%%".to_string());
    let entries = collect_catalog_entries("model", &windows, &compacts, "");
    assert_eq!(entries[0].auto_compact_percent, None);
}
