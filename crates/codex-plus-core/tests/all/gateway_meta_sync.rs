//! #2807：从供应商 `/models` 的元数据同步上下文窗口和思考等级。
//!
//! - 解析：各家字段形态（OpenAI 兼容、OpenRouter、Copilot、Gemini、`context_windows` 档位数组），
//!   异常值（0、负数、超大、类型不对、认不出的档位）跳过不报错；
//! - 差异：只列和现值不同的，分清手动设置 / 上次同步 / 供应商默认 / 内置资料；
//! - 写入：只写勾选的，不在列表里的模型、不合理的值跳过；手动值默认不勾，勾了才覆盖；
//! - 生效：窗口进 Codex 托管供应商的模型列表后缀，思考等级进它的模型目录资料。

use std::collections::BTreeMap;

use codex_plus_core::gateway::apply::{codex_gateway_profile, set_provider_windows};
use codex_plus_core::gateway::meta_sync::{
    BuiltinMeta, CurrentSource, ReasoningPick, WindowPick, apply_reasoning_sync, apply_window_sync,
    build_preview, upstream_model_metas,
};
use codex_plus_core::gateway::store::{
    AgentBinding, GatewayProvider, GatewaySettings, GatewayStore, new_client_token,
};
use codex_plus_core::settings::{RelayMode, RelayProfile, RelayProtocol};
use codex_plus_core::tools::ToolId;
use codex_plus_core::upstream_model_meta::{IgnoredReason, parse_upstream_model_metas};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn provider(id: &str, models: &str) -> GatewayProvider {
    GatewayProvider {
        enabled: true,
        source: String::new(),
        price: None,
        model_prices: Default::default(),
        model_windows: Default::default(),
        context_window: None,
        proxy: String::new(),
        key_rpm: None,
        key_concurrency: None,
        upstream_new: Vec::new(),
        upstream_gone: Vec::new(),
        models_checked_at_ms: 0,
        newapi_access: None,
        upstream_meta: Default::default(),
        relay: RelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            relay_mode: RelayMode::PureApi,
            protocol: RelayProtocol::Responses,
            base_url: "https://meta-sync.test/v1".to_string(),
            api_key: format!("sk-{id}"),
            model_list: models.to_string(),
            ..RelayProfile::default()
        },
        extra: Default::default(),
    }
}

fn store_with(provider: GatewayProvider) -> (tempfile::TempDir, GatewayStore) {
    let temp = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(temp.path().join("gateway.json"));
    let gateway = GatewaySettings {
        providers: vec![provider],
        auto_models: false,
        ..GatewaySettings::default()
    };
    store.save(&gateway).unwrap();
    (temp, store)
}

/// issue 里给的那种中转站响应。
fn issue_payload() -> Value {
    json!({
        "object": "list",
        "data": [{
            "id": "gpt-x",
            "object": "model",
            "context_length": 1000000,
            "max_input_tokens": 1000000,
            "max_output_tokens": 384000,
            "max_completion_tokens": 384000,
            "context_windows": [300000, 600000, 1000000],
            "reasoning_efforts": ["low", "high", "max"],
            "reasoning_default_effort": "high",
            "credits": "x0.11",
            "vendor": "f",
            "temperature": 1
        }]
    })
}

#[test]
fn parses_the_issue_payload_with_sources() {
    let metas = parse_upstream_model_metas(&issue_payload());
    assert_eq!(metas.len(), 1);
    let meta = &metas[0];
    assert_eq!(meta.model, "gpt-x");
    assert_eq!(meta.context_window, Some(1_000_000));
    assert_eq!(meta.context_window_source, "context_length");
    assert_eq!(
        meta.context_window_options,
        vec![300_000, 600_000, 1_000_000]
    );
    assert_eq!(meta.reasoning_levels, vec!["low", "high", "max"]);
    assert_eq!(meta.reasoning_levels_source, "reasoning_efforts");
    assert_eq!(meta.default_reasoning_level.as_deref(), Some("high"));
    assert_eq!(meta.default_reasoning_source, "reasoning_default_effort");
    assert!(meta.ignored.is_empty(), "{:?}", meta.ignored);
}

#[test]
fn parses_common_vendor_shapes() {
    let payload = json!({
        "data": [
            // OpenRouter：顶层没有窗口时取 top_provider.context_length。
            { "id": "openrouter/model", "top_provider": { "context_length": 200000 } },
            // Copilot：capabilities.limits / supports。
            { "id": "copilot-model", "capabilities": {
                "limits": { "max_context_window_tokens": 264000, "max_prompt_tokens": 128000 },
                "supports": { "reasoning_effort": ["low", "medium", "high", "xhigh"] }
            } },
            // Codex 目录风格：supported_reasoning_levels 是对象数组。
            { "id": "catalog-style", "context_window": "272K",
              "supported_reasoning_levels": [{ "effort": "medium" }, { "effort": "high" }],
              "default_reasoning_level": "medium" },
            // 只有 context_windows：取最大值。
            { "id": "tiers-only", "context_windows": [128000, "256k"] },
            // 只有输入上限 + 嵌套 reasoning。
            { "id": "input-only", "max_input_tokens": 400000.0,
              "reasoning": { "efforts": "low, X-High", "default": "x-high" } },
        ]
    });
    let metas = parse_upstream_model_metas(&payload);
    let get = |id: &str| metas.iter().find(|meta| meta.model == id).unwrap();
    assert_eq!(get("openrouter/model").context_window, Some(200_000));
    assert_eq!(
        get("openrouter/model").context_window_source,
        "top_provider.context_length"
    );
    let copilot = get("copilot-model");
    assert_eq!(copilot.context_window, Some(264_000));
    assert_eq!(
        copilot.context_window_source,
        "capabilities.limits.max_context_window_tokens"
    );
    assert_eq!(
        copilot.reasoning_levels,
        vec!["low", "medium", "high", "xhigh"]
    );
    let catalog = get("catalog-style");
    assert_eq!(catalog.context_window, Some(272_000));
    assert_eq!(catalog.reasoning_levels, vec!["medium", "high"]);
    assert_eq!(catalog.default_reasoning_level.as_deref(), Some("medium"));
    assert_eq!(get("tiers-only").context_window, Some(256_000));
    assert_eq!(get("tiers-only").context_window_source, "context_windows");
    let input = get("input-only");
    assert_eq!(input.context_window, Some(400_000));
    assert_eq!(input.context_window_source, "max_input_tokens");
    assert_eq!(input.reasoning_levels, vec!["low", "xhigh"]);
    assert_eq!(input.default_reasoning_level.as_deref(), Some("xhigh"));

    // Gemini：`models` 数组，名字带 `models/` 前缀，窗口是 inputTokenLimit。
    let gemini = parse_upstream_model_metas(&json!({
        "models": [{ "name": "models/gemini-3-pro", "inputTokenLimit": 1048576 }]
    }));
    assert_eq!(gemini[0].model, "gemini-3-pro");
    assert_eq!(gemini[0].context_window, Some(1_048_576));
    assert_eq!(gemini[0].context_window_source, "inputTokenLimit");
}

#[test]
fn skips_invalid_values_without_failing() {
    let payload = json!([
        { "id": "zero", "context_length": 0, "max_input_tokens": 128000 },
        { "id": "negative", "context_window": -1 },
        { "id": "huge", "context_length": 1_000_000_000_000u64 },
        { "id": "text", "context_length": "lots", "reasoning_efforts": "nope" },
        { "id": "wrong-type", "context_length": true, "reasoning_efforts": 3,
          "context_windows": "1M" },
        { "id": "levels", "reasoning_efforts": ["low", "turbo", 5, "HIGH"],
          "reasoning_default_effort": "medium" },
        { "id": "only-unknown", "reasoning_efforts": ["turbo"], "default_reasoning_effort": "turbo" },
        { "id": "fraction", "context_length": 1000.5 },
        { "object": "no id here", "context_length": 100000 },
        "plain-string-model",
        { "id": "LEVELS", "context_length": 64000 }
    ]);
    let metas = parse_upstream_model_metas(&payload);
    let get = |id: &str| metas.iter().find(|meta| meta.model == id).unwrap();
    // 0 不合理，退到下一个字段。
    assert_eq!(get("zero").context_window, Some(128_000));
    assert_eq!(get("zero").ignored[0].reason, IgnoredReason::InvalidWindow);
    for id in ["negative", "huge", "text", "wrong-type", "fraction"] {
        assert_eq!(get(id).context_window, None, "{id}");
        assert!(
            get(id)
                .ignored
                .iter()
                .any(|note| note.reason == IgnoredReason::InvalidWindow),
            "{id}: {:?}",
            get(id).ignored
        );
    }
    assert!(
        get("wrong-type")
            .ignored
            .iter()
            .any(|note| note.field == "reasoning_efforts"
                && note.reason == IgnoredReason::InvalidType)
    );
    assert!(
        get("wrong-type").ignored.iter().any(
            |note| note.field == "context_windows" && note.reason == IgnoredReason::InvalidType
        )
    );
    // 认得的留下（按由低到高），认不出的记下；默认档不在档位里的丢掉。
    let levels = get("levels");
    assert_eq!(levels.reasoning_levels, vec!["low", "high"]);
    assert_eq!(levels.default_reasoning_level, None);
    let reasons: Vec<_> = levels.ignored.iter().map(|note| note.reason).collect();
    assert_eq!(
        reasons,
        vec![
            IgnoredReason::UnknownLevel,
            IgnoredReason::UnknownLevel,
            IgnoredReason::DefaultNotInLevels
        ]
    );
    assert!(get("only-unknown").reasoning_levels.is_empty());
    assert_eq!(get("only-unknown").default_reasoning_level, None);
    // 没有模型名的条目跳过，同名（忽略大小写）只取第一个。
    assert!(metas.iter().all(|meta| meta.model != "no id here"));
    assert_eq!(
        metas
            .iter()
            .filter(|meta| meta.model.eq_ignore_ascii_case("levels"))
            .count(),
        1
    );
    // 完全不像模型列表的响应：空结果，不 panic。
    assert!(parse_upstream_model_metas(&json!({ "error": "x" })).is_empty());
    assert!(parse_upstream_model_metas(&json!(42)).is_empty());
}

fn builtin(
    entries: &[(&str, Option<u64>, &[&str], Option<&str>)],
) -> BTreeMap<String, BuiltinMeta> {
    entries
        .iter()
        .map(|(model, window, levels, default)| {
            (
                model.to_string(),
                BuiltinMeta {
                    window: *window,
                    levels: levels.iter().map(|level| level.to_string()).collect(),
                    default_level: default.map(str::to_string),
                },
            )
        })
        .collect()
}

#[test]
fn preview_lists_only_differences_and_marks_manual_values() {
    let mut provider = provider(
        "p",
        "builtin-m\ncustom-m\nsynced-m\nfallback-m\nsame-m[200K]\nnot-upstream",
    );
    provider.model_windows.insert("Custom-M".into(), 128_000);
    provider.model_windows.insert("synced-m".into(), 300_000);
    provider.model_windows.insert("same-m".into(), 200_000);
    provider
        .upstream_meta
        .windows
        .insert("synced-m".into(), 300_000);
    provider.relay.model_metadata = json!({
        "custom-m": { "supported_reasoning_levels": [{ "effort": "high" }], "default_reasoning_level": "high" },
        "synced-m": { "supported_reasoning_levels": [{ "effort": "low" }, { "effort": "high" }] },
        "fallback-m": { "codex_plus_reasoning_parameters": "omit" }
    })
    .to_string();
    provider.upstream_meta.reasoning.insert(
        "synced-m".into(),
        codex_plus_core::gateway::store::SyncedReasoning {
            levels: vec!["low".into(), "high".into()],
            default_level: None,
        },
    );
    let mut fallback_provider = provider.clone();
    provider.context_window = None;
    fallback_provider.context_window = Some(64_000);

    let upstream = parse_upstream_model_metas(&json!({ "data": [
        { "id": "BUILTIN-M", "context_length": 1000000, "reasoning_efforts": ["low", "high", "max"], "reasoning_default_effort": "high" },
        { "id": "custom-m", "context_length": 400000, "reasoning_efforts": ["low", "high"] },
        { "id": "synced-m", "context_length": 600000, "reasoning_efforts": ["low", "high", "xhigh"] },
        { "id": "fallback-m", "context_length": 256000, "reasoning_efforts": ["medium"] },
        { "id": "same-m", "context_length": 200000, "reasoning_efforts": ["low", "medium"], "reasoning_default_effort": "turbo" },
        { "id": "only-upstream", "context_length": 999000 }
    ] }));
    let builtin = builtin(&[
        (
            "builtin-m",
            Some(272_000),
            &["low", "medium", "high"],
            Some("medium"),
        ),
        ("fallback-m", Some(128_000), &["low"], None),
        ("same-m", None, &["low", "medium"], None),
    ]);

    let preview = build_preview(&provider, &upstream, &builtin);
    assert_eq!(preview.upstream_models, 6);
    assert_eq!(
        preview.matched, 5,
        "not-upstream 上游没有，only-upstream 不在列表里"
    );
    assert_eq!(preview.with_window, 5);
    let window = |model: &str| preview.windows.iter().find(|change| change.model == model);
    let builtin_window = window("builtin-m").unwrap();
    assert_eq!(
        (
            builtin_window.current,
            builtin_window.current_source,
            builtin_window.manual
        ),
        (Some(272_000), CurrentSource::Builtin, false)
    );
    assert_eq!(builtin_window.window, 1_000_000);
    assert_eq!(builtin_window.source, "context_length");
    let custom = window("custom-m").unwrap();
    assert_eq!(
        (custom.current, custom.current_source, custom.manual),
        (Some(128_000), CurrentSource::Custom, true)
    );
    let synced = window("synced-m").unwrap();
    assert_eq!(
        (synced.current_source, synced.manual),
        (CurrentSource::Synced, false)
    );
    assert_eq!(
        window("fallback-m").unwrap().current_source,
        CurrentSource::Builtin
    );
    assert!(
        window("same-m").is_none(),
        "窗口相同的不列（列表里的 [200K] 后缀就是现值）"
    );

    let reasoning = |model: &str| {
        preview
            .reasoning
            .iter()
            .find(|change| change.model == model)
    };
    let builtin_levels = reasoning("builtin-m").unwrap();
    assert_eq!(builtin_levels.current_levels, vec!["low", "medium", "high"]);
    assert_eq!(builtin_levels.current_source, CurrentSource::Builtin);
    assert_eq!(builtin_levels.levels, vec!["low", "high", "max"]);
    assert_eq!(builtin_levels.default_level.as_deref(), Some("high"));
    assert!(!builtin_levels.manual);
    let custom_levels = reasoning("custom-m").unwrap();
    assert_eq!(
        (custom_levels.current_source, custom_levels.manual),
        (CurrentSource::Custom, true)
    );
    assert_eq!(custom_levels.current_default.as_deref(), Some("high"));
    assert_eq!(
        reasoning("synced-m").unwrap().current_source,
        CurrentSource::Synced
    );
    assert!(!reasoning("synced-m").unwrap().manual);
    // 选了「思考参数：不发送」也算手动设置。
    let omitted = reasoning("fallback-m").unwrap();
    assert_eq!(
        (omitted.current_source, omitted.manual),
        (CurrentSource::Custom, true)
    );
    // 档位一样、上游默认档认不出被丢掉：不算差异，但跳过的字段要提示。
    assert!(reasoning("same-m").is_none());
    assert!(
        preview
            .ignored
            .iter()
            .any(|note| note.model == "same-m" && note.field == "reasoning_default_effort")
    );

    // 供应商有默认窗口：没单独写的模型以它为现值（不算手动）。
    let preview = build_preview(&fallback_provider, &upstream, &builtin);
    let fallback = preview
        .windows
        .iter()
        .find(|change| change.model == "fallback-m")
        .unwrap();
    assert_eq!(
        (fallback.current, fallback.current_source, fallback.manual),
        (Some(64_000), CurrentSource::ProviderDefault, false)
    );
}

#[test]
fn applying_windows_writes_only_picked_and_tracks_synced_values() {
    let mut start = provider("w", "gpt-x\nmine[128K]\nother");
    start.model_windows.insert("MINE".into(), 128_000);
    start.model_windows.insert("other".into(), 64_000);
    let (_temp, store) = store_with(start);

    let (saved, written) = apply_window_sync(
        &store,
        "w",
        &[
            WindowPick {
                model: "GPT-X".into(),
                window: 1_000_000,
            },
            WindowPick {
                model: "not-listed".into(),
                window: 500_000,
            },
            WindowPick {
                model: "other".into(),
                window: 0,
            },
            WindowPick {
                model: "other".into(),
                window: 1_000_000_000_000,
            },
        ],
    )
    .unwrap();
    assert_eq!(written, 1);
    let provider = saved.provider("w").unwrap();
    assert_eq!(provider.window_for("gpt-x"), Some(1_000_000));
    assert_eq!(
        provider.model_windows.get("MINE"),
        Some(&128_000),
        "没勾的手动值不动"
    );
    assert_eq!(provider.model_windows.get("other"), Some(&64_000));
    assert!(!provider.model_windows.contains_key("not-listed"));
    assert_eq!(
        provider.upstream_meta.windows.get("gpt-x"),
        Some(&1_000_000)
    );

    // 勾了手动值才覆盖：同名旧记录（大小写不同）被替换，不留两份。
    let (saved, written) = apply_window_sync(
        &store,
        "w",
        &[WindowPick {
            model: "mine[128K]".into(),
            window: 400_000,
        }],
    )
    .unwrap();
    assert_eq!(written, 1);
    let provider = saved.provider("w").unwrap();
    assert_eq!(provider.window_for("mine"), Some(400_000));
    assert_eq!(
        provider
            .model_windows
            .keys()
            .filter(|key| key.eq_ignore_ascii_case("mine"))
            .count(),
        1
    );

    // 同步来的值下次预览算「上次同步」；用户在窗口弹窗里改过就成了手动值。
    let upstream =
        parse_upstream_model_metas(&json!([{ "id": "gpt-x", "context_length": 2000000 }]));
    let preview = build_preview(saved.provider("w").unwrap(), &upstream, &BTreeMap::new());
    assert_eq!(preview.windows[0].current_source, CurrentSource::Synced);
    assert!(!preview.windows[0].manual);
    let mut windows = saved.provider("w").unwrap().model_windows.clone();
    windows.insert("gpt-x".into(), 900_000);
    let saved = set_provider_windows(&store, "w", windows, None).unwrap();
    let preview = build_preview(saved.provider("w").unwrap(), &upstream, &BTreeMap::new());
    assert_eq!(preview.windows[0].current_source, CurrentSource::Custom);
    assert!(preview.windows[0].manual);

    // 读盘后记录还在。
    let loaded = store.load().unwrap();
    assert_eq!(
        loaded
            .provider("w")
            .unwrap()
            .upstream_meta
            .windows
            .get("gpt-x"),
        Some(&1_000_000)
    );
}

#[test]
fn applying_reasoning_replaces_levels_and_keeps_other_metadata() {
    let mut start = provider("r", "gpt-x\nmine\nkeep");
    start.relay.model_metadata = json!({
        "Mine": {
            "supported_reasoning_levels": [{ "effort": "high", "description": "x" }],
            "default_reasoning_level": "high",
            "codex_plus_reasoning_parameters": "passthrough",
            "supports_search_tool": false
        },
        "keep": { "default_reasoning_level": "low", "supported_reasoning_levels": [{ "effort": "low" }] }
    })
    .to_string();
    let (_temp, store) = store_with(start);

    let (saved, written) = apply_reasoning_sync(
        &store,
        "r",
        &[
            ReasoningPick {
                model: "gpt-x".into(),
                levels: vec!["max".into(), "low".into(), "turbo".into(), "high".into()],
                default_level: Some("high".into()),
            },
            // 上游没给默认档：原来的 high 还在新档位里，保留。
            ReasoningPick {
                model: "MINE".into(),
                levels: vec!["low".into(), "high".into()],
                default_level: None,
            },
            ReasoningPick {
                model: "not-listed".into(),
                levels: vec!["low".into()],
                default_level: None,
            },
            ReasoningPick {
                model: "keep".into(),
                levels: vec!["turbo".into()],
                default_level: None,
            },
        ],
    )
    .unwrap();
    assert_eq!(written, 2);
    let provider = saved.provider("r").unwrap();
    let metadata: Value = serde_json::from_str(&provider.relay.model_metadata).unwrap();
    let efforts = |key: &str| -> Vec<String> {
        metadata[key]["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|level| level["effort"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(efforts("gpt-x"), vec!["low", "high", "max"]);
    assert_eq!(metadata["gpt-x"]["default_reasoning_level"], "high");
    assert_eq!(
        metadata["gpt-x"]["supported_reasoning_levels"][0]["description"],
        "Fast responses with lighter reasoning"
    );
    // 沿用原来的键，其它字段原样保留。
    assert_eq!(efforts("Mine"), vec!["low", "high"]);
    assert_eq!(metadata["Mine"]["default_reasoning_level"], "high");
    assert_eq!(
        metadata["Mine"]["codex_plus_reasoning_parameters"],
        "passthrough"
    );
    assert_eq!(metadata["Mine"]["supports_search_tool"], false);
    assert!(metadata.get("mine").is_none());
    assert_eq!(efforts("keep"), vec!["low"], "全是认不出的档位：跳过不写");
    assert!(metadata.get("not-listed").is_none());
    assert_eq!(
        provider.upstream_meta.reasoning["gpt-x"].levels,
        vec!["low", "high", "max"]
    );
    assert_eq!(
        provider.upstream_meta.reasoning["mine"]
            .default_level
            .as_deref(),
        Some("high")
    );

    // 默认档不在新档位里：去掉。
    let (saved, _) = apply_reasoning_sync(
        &store,
        "r",
        &[ReasoningPick {
            model: "mine".into(),
            levels: vec!["low".into()],
            default_level: Some("max".into()),
        }],
    )
    .unwrap();
    let metadata: Value =
        serde_json::from_str(&saved.provider("r").unwrap().relay.model_metadata).unwrap();
    assert!(metadata["Mine"].get("default_reasoning_level").is_none());

    // 同步后再预览：现值算「上次同步」。
    let upstream = parse_upstream_model_metas(
        &json!([{ "id": "gpt-x", "reasoning_efforts": ["low", "medium"] }]),
    );
    let preview = build_preview(saved.provider("r").unwrap(), &upstream, &BTreeMap::new());
    assert_eq!(preview.reasoning[0].current_source, CurrentSource::Synced);
}

#[test]
fn applying_reasoning_refuses_broken_metadata() {
    let mut start = provider("broken", "gpt-x");
    start.relay.model_metadata = "{not json".into();
    let (_temp, store) = store_with(start);
    assert!(
        apply_reasoning_sync(
            &store,
            "broken",
            &[ReasoningPick {
                model: "gpt-x".into(),
                levels: vec!["low".into()],
                default_level: None
            }]
        )
        .is_err()
    );
    assert_eq!(
        store
            .load()
            .unwrap()
            .provider("broken")
            .unwrap()
            .relay
            .model_metadata,
        "{not json"
    );
}

#[test]
fn synced_values_reach_the_codex_gateway_profile() {
    let (_temp, store) = store_with(provider("eff", "gpt-x\nother"));
    apply_window_sync(
        &store,
        "eff",
        &[WindowPick {
            model: "gpt-x".into(),
            window: 1_000_000,
        }],
    )
    .unwrap();
    let (mut gateway, _) = apply_reasoning_sync(
        &store,
        "eff",
        &[ReasoningPick {
            model: "gpt-x".into(),
            levels: vec!["low".into(), "high".into(), "max".into()],
            default_level: Some("high".into()),
        }],
    )
    .unwrap();
    gateway.bindings.insert(
        "codex".to_string(),
        AgentBinding {
            enabled: true,
            model: "gpt-x".to_string(),
            token: new_client_token(&ToolId::Codex),
            ..AgentBinding::default()
        },
    );
    let binding = gateway.bindings["codex"].clone();
    let profile = codex_gateway_profile(&gateway, &binding, 57390, false).unwrap();
    // 模型列表带的 `[窗口]` 后缀在归一化时收进托管供应商的 model_windows，生成 catalog 时写进 context_window。
    let windows: BTreeMap<String, String> = serde_json::from_str(&profile.model_windows).unwrap();
    assert_eq!(
        windows.get("gpt-x").map(String::as_str),
        Some("1000000"),
        "{}",
        profile.model_windows
    );
    assert!(!windows.contains_key("other"), "没同步的模型不带窗口");
    let metadata: Value = serde_json::from_str(&profile.model_metadata).unwrap();
    let entry = metadata
        .as_object()
        .unwrap()
        .iter()
        .find(|(model, _)| model.as_str() == "gpt-x")
        .map(|(_, entry)| entry)
        .expect("Codex 模型目录里有 gpt-x");
    let efforts: Vec<&str> = entry["supported_reasoning_levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|level| level["effort"].as_str().unwrap())
        .collect();
    assert_eq!(efforts, vec!["low", "high", "max"]);
    assert_eq!(entry["default_reasoning_level"], "high");
}

#[tokio::test]
async fn reads_metadata_through_the_models_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_payload()))
        .mount(&server)
        .await;
    let mut relay = provider("fetch", "gpt-x").relay;
    relay.base_url = format!("{}/v1", server.uri());
    let metas = upstream_model_metas(&relay).await.unwrap();
    assert_eq!(metas[0].context_window, Some(1_000_000));
    assert_eq!(metas[0].reasoning_levels, vec!["low", "high", "max"]);

    // 上游报错：带出原因，不返回空结果。
    let failing = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(401).set_body_json(json!({ "error": { "message": "bad key" } })),
        )
        .mount(&failing)
        .await;
    relay.base_url = format!("{}/v1", failing.uri());
    let error = upstream_model_metas(&relay).await.unwrap_err();
    assert!(format!("{error:#}").contains("bad key"), "{error:#}");
}
