use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::Mutex;

use codex_plus_core::model_suffix::{
    build_model_catalog_json, build_model_catalog_json_with_template, builtin_model_metadata,
    builtin_model_metadata_index, collect_catalog_entries, model_ui_metadata, parse_model_suffix,
};

/// CODEX_HOME 环境变量是进程级全局，运行时缓存测试必须串行执行。
static RUNTIME_CACHE_ENV_LOCK: Mutex<()> = Mutex::new(());

/// 保存并恢复 CODEX_HOME 的守卫，参照 codex_home.rs 内部测试的模式。
struct CodexHomeEnvGuard {
    previous: Option<OsString>,
}

impl CodexHomeEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("CODEX_HOME");
        unsafe {
            std::env::set_var("CODEX_HOME", path);
        }
        Self { previous }
    }
}

impl Drop for CodexHomeEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("CODEX_HOME", value),
                None => std::env::remove_var("CODEX_HOME"),
            }
        }
    }
}

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
    let entries = collect_catalog_entries(
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
    let entries = collect_catalog_entries(
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
        // 官方 gpt-5.6 目录为 272000/872000：未显式配置窗口时保留官方上限（issue #2191）。
        assert_eq!(model["max_context_window"], 872_000);
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
fn astra_metadata_exposes_max_ultra_in_catalog_and_ui() {
    use codex_plus_core::model_suffix::requires_bundled_metadata_catalog;

    assert!(requires_bundled_metadata_catalog("gpt-6-astra"));
    assert!(!requires_bundled_metadata_catalog("gpt-6-astra-custom"));
    assert!(model_ui_metadata("gpt-6-astra-custom").is_none());
    let entries = collect_catalog_entries("gpt-6-astra", &HashMap::new(), &HashMap::new(), "");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    let model = &catalog["models"][0];
    let metadata = model_ui_metadata("gpt-6-astra").unwrap();
    let expected = vec!["low", "medium", "high", "xhigh", "max", "ultra"];
    for (levels, key) in [
        (&model["supported_reasoning_levels"], "effort"),
        (&metadata["supportedReasoningEfforts"], "reasoningEffort"),
    ] {
        let efforts: Vec<_> = levels
            .as_array()
            .unwrap()
            .iter()
            .map(|level| level[key].as_str().unwrap())
            .collect();
        assert_eq!(efforts, expected);
    }
    assert_eq!(model["display_name"], "GPT-6-Astra");
    assert_eq!(metadata["displayName"], model["display_name"]);
    assert_eq!(model["default_reasoning_level"], "medium");
    assert_eq!(metadata["defaultReasoningEffort"], "medium");
    assert_eq!(model["context_window"], 272_000);
    // 官方 gpt-6-astra 目录为 272000/872000：未显式配置窗口时保留官方上限（issue #2191）。
    assert_eq!(model["max_context_window"], 872_000);
    assert_eq!(model["supports_search_tool"], true);
    assert_eq!(model["supports_image_detail_original"], true);
    assert_eq!(model["use_responses_lite"], false);
    assert_eq!(model["additional_speed_tiers"], serde_json::json!(["fast"]));
    assert_eq!(
        metadata["additionalSpeedTiers"],
        model["additional_speed_tiers"]
    );
    assert_eq!(model["service_tiers"][0]["id"], "priority");
    assert_eq!(model["service_tiers"][0]["name"], "Fast");
    assert_eq!(metadata["serviceTiers"], model["service_tiers"]);

    let overridden: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, Some(200_000))).unwrap();
    assert_eq!(overridden["models"][0]["context_window"], 200_000);
    // 显式配置窗口（profile 全局 / 每模型）时两字段同值，产品语义不变（issue #2191）。
    assert_eq!(overridden["models"][0]["max_context_window"], 200_000);
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
    let entries = collect_catalog_entries(
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
fn build_catalog_json_prefers_runtime_models_cache_entry() {
    let _lock = RUNTIME_CACHE_ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let cache = serde_json::json!({
        "models": [{
            "slug": "gpt-5.5",
            "display_name": "GPT-5.5 Runtime",
            "description": "runtime cache wins",
            "context_window": 272000u64,
            "max_context_window": 872000u64,
            "shell_type": "unified_exec"
        }]
    });
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(
        temp.path().join("models_cache.json"),
        serde_json::to_string(&cache).unwrap(),
    )
    .unwrap();
    let _guard = CodexHomeEnvGuard::set(temp.path());

    let entries = collect_catalog_entries("gpt-5.5", &HashMap::new(), &HashMap::new(), "");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    let model = &catalog["models"][0];

    assert_eq!(model["slug"], "gpt-5.5");
    // 运行时缓存命中：display_name / shell_type 来自 models_cache.json（issue #2141）
    assert_eq!(model["display_name"], "GPT-5.5 Runtime");
    assert_eq!(model["shell_type"], "unified_exec");
    // 窗口仍由 builder 权威生成：未显式配置时保留缓存的上限（#2191 语义）
    assert_eq!(model["context_window"], 272_000);
    assert_eq!(model["max_context_window"], 872_000);
}

#[test]
fn build_catalog_json_falls_back_to_bundled_without_runtime_cache() {
    let _lock = RUNTIME_CACHE_ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(&temp).unwrap();
    let _guard = CodexHomeEnvGuard::set(temp.path());

    let entries = collect_catalog_entries("gpt-5.5", &HashMap::new(), &HashMap::new(), "");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    let model = &catalog["models"][0];

    // 无运行时缓存时回落静态资产，字段仍然齐全
    assert_eq!(model["slug"], "gpt-5.5");
    assert_eq!(model["display_name"], "GPT-5.5");
    assert_eq!(model["context_window"], 272_000);
    assert!(model["supported_reasoning_levels"].as_array().is_some());
}

#[test]
fn catalog_metadata_matches_slug_case_insensitively() {
    use codex_plus_core::model_suffix::requires_bundled_metadata_catalog;

    // 供应商 Model Key 大小写不统一（GLM-5.3-FlashX），界面填大写也应命中内置元数据。
    assert!(requires_bundled_metadata_catalog("GPT-6-Astra"));
    assert!(!requires_bundled_metadata_catalog("gpt-6-astra-custom"));
    let metadata = model_ui_metadata("GPT-5.6-SOL").expect("Sol metadata should exist");
    assert_eq!(metadata["displayName"], "GPT-5.6-Sol");
}

#[test]
fn bundled_template_matches_slug_case_insensitively() {
    // codex bundled catalog 全小写 slug，界面填大写（GPT-5.4）也应命中模板：
    // 命中时保留官方 max_context_window 1000000，未命中回落首条模板的 272000。
    // 隔离 CODEX_HOME：否则查找链会先命中本机官方 models_cache.json，机器相关。
    let _lock = RUNTIME_CACHE_ENV_LOCK.lock().unwrap();
    let _guard = CodexHomeEnvGuard::set(tempfile::tempdir().unwrap().path());
    let entries = collect_catalog_entries("GPT-5.4", &HashMap::new(), &HashMap::new(), "GPT-5.4");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    assert_eq!(catalog["models"][0]["slug"], "GPT-5.4");
    assert_eq!(catalog["models"][0]["context_window"], 272_000);
    assert_eq!(catalog["models"][0]["max_context_window"], 1_000_000);
}

#[test]
fn vendor_metadata_chain_matches_all_providers() {
    use codex_plus_core::model_suffix::requires_bundled_metadata_catalog;

    // 未配置窗口时，供应商元数据直接提供窗口与展示字段（#2191：未显式配置
    // 保留官方 max 上限），不再回落 272000 裸模板。
    assert!(requires_bundled_metadata_catalog("kimi-k3"));
    let entries = collect_catalog_entries(
        "kimi-k3\nglm-5.3\nqwen3.8-max\ngrok-4.7\nMiniMax-M3",
        &HashMap::new(),
        &HashMap::new(),
        "",
    );
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    let models = catalog["models"].as_array().unwrap();
    let find = |slug: &str| {
        models
            .iter()
            .find(|model| model["slug"] == slug)
            .expect(slug)
            .clone()
    };

    let k3 = find("kimi-k3");
    assert_eq!(k3["display_name"], "Kimi K3");
    assert_eq!(k3["context_window"], 1_048_576);
    assert_eq!(k3["max_context_window"], 1_048_576);
    assert_eq!(k3["apply_patch_tool_type"], "freeform");
    let efforts = effort_list(&k3);
    assert_eq!(efforts, vec!["low", "high", "max"]);

    let glm = find("glm-5.3");
    assert_eq!(glm["display_name"], "glm-5.3");
    assert_eq!(glm["context_window"], 1_048_576);
    assert_eq!(effort_list(&glm), vec!["low", "high", "max"]);

    let qwen = find("qwen3.8-max");
    assert_eq!(qwen["context_window"], 1_000_000);
    assert_eq!(qwen["max_context_window"], 1_000_000);
    assert_eq!(effort_list(&qwen), vec!["low", "medium", "xhigh"]);

    let grok = find("grok-4.7");
    assert_eq!(grok["display_name"], "Grok 4.7");
    assert_eq!(grok["context_window"], 500_000);
    assert_eq!(effort_list(&grok), vec!["low", "medium", "high", "xhigh"]);

    let minimax = find("MiniMax-M3");
    assert_eq!(minimax["display_name"], "MiniMax-M3");
    assert_eq!(minimax["context_window"], 1_048_576);
    assert_eq!(effort_list(&minimax), vec!["none", "high"]);
}

fn effort_list(model: &serde_json::Value) -> Vec<&str> {
    model["supported_reasoning_levels"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|level| level["effort"].as_str())
        .collect()
}

#[test]
fn vendor_metadata_chain_matches_case_insensitively() {
    // 界面填大写供应商 slug（GLM-5.3）也应命中内置供应商元数据。
    let entries = collect_catalog_entries("GLM-5.3", &HashMap::new(), &HashMap::new(), "GLM-5.3");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    assert_eq!(catalog["models"][0]["slug"], "GLM-5.3");
    assert_eq!(catalog["models"][0]["display_name"], "glm-5.3");
    assert_eq!(catalog["models"][0]["context_window"], 1_048_576);
}

#[test]
fn compat_overlay_composes_with_runtime_cache_base() {
    // 精调层与官方 App 热更新的冲突裁决：官方缓存做基座（未被精调覆盖的字段
    // 流入官方最新值），精调字段覆盖其上（产品特性不被官方数据冲掉）。
    let _lock = RUNTIME_CACHE_ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let cache = serde_json::json!({
        "models": [{
            "slug": "gpt-5.6-sol",
            "display_name": "GPT-5.6 Sol Runtime",
            "context_window": 400_000u64,
            "max_context_window": 400_000u64,
            "shell_type": "shell_command",
            // 精调文件未定义的字段：官方热更新应原样流入
            "truncation_policy": { "mode": "tokens", "limit": 999 }
        }]
    });
    std::fs::create_dir_all(temp.path()).unwrap();
    std::fs::write(temp.path().join("models_cache.json"), cache.to_string()).unwrap();
    let _guard = CodexHomeEnvGuard::set(temp.path());

    let entries = collect_catalog_entries("gpt-5.6-sol", &HashMap::new(), &HashMap::new(), "");
    let catalog: serde_json::Value =
        serde_json::from_str(&build_model_catalog_json(&entries, None)).unwrap();
    let model = &catalog["models"][0];
    // 精调覆盖：窗口与 Fast 档保持产品级取值
    assert_eq!(model["context_window"], 272_000);
    assert_eq!(model["max_context_window"], 872_000);
    assert_eq!(model["additional_speed_tiers"], serde_json::json!(["fast"]));
    // 官方流入：精调未定义的截断策略取缓存最新值
    assert_eq!(model["truncation_policy"]["limit"], 999);
}

#[test]
fn builtin_model_metadata_matches_with_source() {

    // 供应商事实层：附带来源名
    let kimi = builtin_model_metadata("kimi-k3").expect("kimi-k3 内置");
    assert_eq!(kimi.source, "Kimi");
    assert_eq!(kimi.entry["display_name"], "Kimi K3");
    // 大小写不敏感 + [1M] 后缀剥除
    let sol = builtin_model_metadata("GPT-5.6-SOL[1M]").expect("gpt-5.6-sol 内置");
    assert_eq!(sol.source, "gpt-5.6 兼容");
    assert_eq!(sol.entry["slug"], "gpt-5.6-sol");
    // 官方 bundled 层
    let official = builtin_model_metadata("gpt-5.5").expect("gpt-5.5 官方内置");
    assert_eq!(official.source, "官方内置");
    // 未命中 → None（生成时回退 gpt-5.5 模板）
    assert!(builtin_model_metadata("unknown-model-xyz").is_none());
    assert!(builtin_model_metadata("").is_none());
}

#[test]
fn builtin_model_metadata_index_covers_all_embedded_entries() {

    let index = builtin_model_metadata_index();
    // 60 条精调/供应商全量 + 官方 bundled 静态资产（随 sync_official_models.py
    // 增长，与精调层重叠的官方 slug 以精调来源优先去重）——不锁死总数，
    // 只断言结构完整与已知覆盖。
    assert!(index.len() >= 60, "内置索引至少覆盖 60 条精调/供应商条目");
    let mut seen = std::collections::HashSet::new();
    for entry in &index {
        let slug = entry["slug"].as_str().expect("slug");
        assert!(seen.insert(slug.to_ascii_lowercase()), "slug 重复：{slug}");
        assert!(entry["source"].as_str().is_some_and(|s| !s.is_empty()));
    }
    for expected in [
        "kimi-k3",
        "gpt-5.6-sol",
        "gpt-6-astra",
        "gpt-5.5",
        "deepseek-v4.1-flash",
    ] {
        assert!(
            seen.contains(&expected.to_ascii_lowercase()),
            "缺少 {expected}"
        );
    }
    let kimi = index.iter().find(|e| e["slug"] == "kimi-k3").unwrap();
    assert_eq!(kimi["source"], "Kimi");
    assert_eq!(kimi["context_window"], 1_048_576);
}

#[test]
fn builtin_model_metadata_hits_runtime_cache_layer() {
    // 运行时官方缓存（models_cache.json）命中的模型：来源标为官方内置，
    // 且预填/查询用运行时的窗口数据而不是静态资产的。
    let _lock = RUNTIME_CACHE_ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path()).unwrap();
    std::fs::write(
        temp.path().join("models_cache.json"),
        serde_json::json!({
            "models": [{
                "slug": "runtime-only-model",
                "display_name": "Runtime Only Model",
                "context_window": 1_048_222u64,
                "max_context_window": 1_048_222u64,
                "truncation_policy": { "mode": "tokens", "limit": 12345 }
            }]
        })
        .to_string(),
    )
    .unwrap();
    let _guard = CodexHomeEnvGuard::set(temp.path());

    let metadata = builtin_model_metadata("runtime-only-model").expect("运行时缓存命中");
    assert_eq!(metadata.source, "官方内置");
    assert_eq!(metadata.entry["display_name"], "Runtime Only Model");
    // 运行时数据（非静态资产）被取到
    assert_eq!(metadata.entry["truncation_policy"]["limit"], 12345);

    // 索引同样收录运行时层 slug
    let index = builtin_model_metadata_index();
    assert!(
        index.iter().any(|e| e["slug"] == "runtime-only-model"),
        "索引应覆盖运行时官方缓存条目"
    );
}

#[test]
fn builtin_model_metadata_index_dedupes_case_variants_and_skips_invalid() {
    // 大小写变体去重 + 无 slug / 非对象条目跳过（脏数据防御）
    let _lock = RUNTIME_CACHE_ENV_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path()).unwrap();
    std::fs::write(
        temp.path().join("models_cache.json"),
        serde_json::json!({
            "models": [
                { "slug": "Glm-5.3", "display_name": "case variant dup" },
                { "display_name": "no slug, must be skipped" },
                "not-an-object"
            ]
        })
        .to_string(),
    )
    .unwrap();
    let _guard = CodexHomeEnvGuard::set(temp.path());

    let index = builtin_model_metadata_index();
    // glm-5.3 静态层先命中（Kimi 等供应商顺序在前），大小写变体被去重
    let glm: Vec<_> = index
        .iter()
        .filter(|e| e["slug"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("glm-5.3")))
        .collect();
    assert_eq!(glm.len(), 1, "大小写变体只应保留首次命中的来源");
    assert_eq!(glm[0]["source"], "GLM");
}
