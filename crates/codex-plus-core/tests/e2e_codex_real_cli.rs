//! E2E：真实 apply 流程 + 真实 codex CLI 解析生成的 model_catalog_json。
//!
//! 仅本机手动执行（需要 codex 二进制，默认取 VS Code ChatGPT 扩展内置版本，
//! 可用 CODEX_E2E_CODEX_BIN 覆盖）：
//!
//! ```text
//! cargo test -p codex-plus-core --test e2e_codex_real_cli -- --ignored --nocapture
//! ```
//!
//! 步骤：
//! 1. 用三家 vendor 元数据（kimi/minimax/qwen，含修复后的 freeform apply_patch）
//!    走 `apply_relay_profile_to_home_with_switch_rules` 生成真实 catalog；
//! 2. 用真实 codex 执行 `debug models`，验证 catalog 被完整解析、全部 slug 可见；
//! 3. 阴性对照：把生成 catalog 里一条 apply_patch_tool_type 改回 "function"，
//!    codex 必须解析失败（0.144+ 的枚举只认 freeform）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules;
use codex_plus_core::settings::{RelayMode, RelayProfile, RelayProtocol};

const KIMI_JSON: &str = include_str!("../../../assets/kimi-model-metadata.json");
const MINIMAX_JSON: &str = include_str!("../../../assets/minimax-model-metadata.json");
const QWEN_JSON: &str = include_str!("../../../assets/qwen-model-metadata.json");

// 不写死本机路径：默认置空，要求显式提供 CODEX_E2E_CODEX_BIN。写死作者机器的
// 绝对路径既对别人无用，也会在扩展升级后变成误导性的失效默认值。
const DEFAULT_CODEX_BIN: &str = "";

/// 镜像前端 filteredMetadata：窗口/压缩四个字段由生成器管辖，不进 metadata map。
fn vendor_metadata_map(vendor_json: &str, map: &mut BTreeMap<String, serde_json::Value>) {
    let catalog: serde_json::Value = serde_json::from_str(vendor_json).unwrap();
    for model in catalog["models"].as_array().unwrap() {
        let slug = model["slug"].as_str().unwrap().to_string();
        let filtered: serde_json::Map<String, serde_json::Value> = model
            .as_object()
            .unwrap()
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "slug" | "context_window" | "max_context_window" | "auto_compact_token_limit"
                )
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        map.insert(slug, serde_json::Value::Object(filtered));
    }
}

fn codex_bin() -> PathBuf {
    std::env::var_os("CODEX_E2E_CODEX_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CODEX_BIN))
}

fn run_codex_debug_models(codex: &PathBuf, home: &std::path::Path) -> (bool, String) {
    let output = Command::new(codex)
        .arg("debug")
        .arg("models")
        .env("CODEX_HOME", home)
        .output()
        .expect("failed to spawn codex");
    let text = format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
#[ignore = "E2E 需要本机 codex 二进制，手动用 --ignored 运行"]
fn e2e_vendor_catalog_loads_in_real_codex() {
    let mut metadata = BTreeMap::new();
    vendor_metadata_map(KIMI_JSON, &mut metadata);
    vendor_metadata_map(MINIMAX_JSON, &mut metadata);
    vendor_metadata_map(QWEN_JSON, &mut metadata);
    let slugs: Vec<&str> = metadata.keys().map(String::as_str).collect();
    // 不写死条数：资产随时会新增/下架模型，写死会让这条唯一「真实 codex 解析」
    // 验证路径在资产正常演进时误红。改用不变量：条数与 catalog 一致、且确实非空，
    // 具体数量只在日志里留痕，方便人工核对漂移幅度。
    assert!(!slugs.is_empty(), "vendor 元数据不应为空");

    let temp = tempfile::tempdir().unwrap();
    let profile = RelayProfile {
        id: "e2e".to_string(),
        name: "E2E Vendor Catalog".to_string(),
        model: "kimi-k3".to_string(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::Responses,
        config_contents: r#"model = "kimi-k3"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://relay.example.test/v1"
"#
        .to_string(),
        auth_contents: r#"{"OPENAI_API_KEY":"sk-e2e-redacted"}"#.to_string(),
        model_list: slugs.join("\n"),
        model_windows: r#"{"kimi-k3":"1048576","qwen3.8-max":"1000000"}"#.to_string(),
        model_metadata: serde_json::to_string(&metadata).unwrap(),
        ..RelayProfile::default()
    };

    apply_relay_profile_to_home_with_switch_rules(temp.path(), &profile, "").unwrap();

    let config = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    let doc: toml_edit::DocumentMut = config.parse().expect("config.toml 应为合法 TOML");
    let catalog_rel = doc["model_catalog_json"]
        .as_str()
        .expect("config.toml 应写入 model_catalog_json 指针")
        .to_string();
    let catalog_path = if PathBuf::from(&catalog_rel).is_absolute() {
        PathBuf::from(&catalog_rel)
    } else {
        temp.path().join(catalog_rel)
    };
    let catalog_text = std::fs::read_to_string(&catalog_path).unwrap();
    let catalog: serde_json::Value = serde_json::from_str(&catalog_text).unwrap();

    let models = catalog["models"].as_array().unwrap();
    assert_eq!(
        models.len(),
        slugs.len(),
        "catalog 条目数应与 vendor 元数据一致"
    );
    for model in models {
        assert_eq!(
            model["apply_patch_tool_type"], "freeform",
            "slug {} 的 apply_patch_tool_type 必须是 freeform",
            model["slug"]
        );
    }
    let k3 = models
        .iter()
        .find(|m| m["slug"] == "kimi-k3")
        .expect("kimi-k3 应在 catalog 中");
    assert_eq!(k3["context_window"], 1_048_576, "每模型窗口应写入 catalog");
    println!(
        "[E2E] catalog 生成 OK：{} 条，apply_patch 全部 freeform，kimi-k3 窗口 1048576",
        models.len()
    );

    // ── 真实 codex 解析 ──
    let codex = codex_bin();
    if codex.as_os_str().is_empty() {
        panic!("请设置 CODEX_E2E_CODEX_BIN 指向本机 codex 二进制，例如：
  CODEX_E2E_CODEX_BIN=/path/to/codex cargo test -p codex-plus-core --test e2e_codex_real_cli -- --ignored --nocapture");
    }
    assert!(codex.exists(), "codex 二进制不存在：{}", codex.display());
    let (ok, text) = run_codex_debug_models(&codex, temp.path());
    assert!(
        ok,
        "真实 codex 解析生成的 catalog 失败：\n{}",
        &text[..text.len().min(4000)]
    );
    for slug in &slugs {
        assert!(text.contains(slug), "codex debug models 输出应包含 {slug}");
    }
    println!(
        "[E2E] codex debug models OK：{} 个 slug 全部被真实 codex 加载",
        slugs.len()
    );

    // ── 阴性对照：一条改回 function，codex 必须拒绝 ──
    let poisoned = catalog_text.replacen(
        "\"apply_patch_tool_type\": \"freeform\"",
        "\"apply_patch_tool_type\": \"function\"",
        1,
    );
    assert_ne!(poisoned, catalog_text, "catalog 应包含 freeform 可供替换");
    std::fs::write(&catalog_path, poisoned).unwrap();
    let (ok2, text2) = run_codex_debug_models(&codex, temp.path());
    assert!(
        !ok2,
        "注入 function 后 codex 仍解析成功，阴性对照失败：\n{}",
        &text2[..text2.len().min(4000)]
    );
    assert!(
        text2.contains("parse") || text2.contains("apply_patch") || text2.contains("variant"),
        "失败信息应指向 catalog 解析：\n{}",
        &text2[..text2.len().min(2000)]
    );
    println!("[E2E] 阴性对照 OK：function 值被真实 codex 拒绝（整份 catalog 解析失败）");
}

/// E2E：精调层与官方 App 运行时缓存（models_cache.json）的组合裁决。
/// A 段用造出来的冲突缓存钉死字段级行为；B 段复制本机真实官方缓存
/// （只读，含 gpt-5.6-terra/luna 等真实重叠 slug）验证官方数据流入，
/// 最后让真实 codex 解析组合出的 catalog。
#[test]
#[ignore = "E2E 需要本机 codex 二进制，手动用 --ignored 运行"]
fn e2e_runtime_cache_base_composes_with_compat_overlay() {
    struct CodexHomeGuard(std::ffi::OsString);
    impl CodexHomeGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("CODEX_HOME");
            unsafe { std::env::set_var("CODEX_HOME", path) };
            Self(previous.unwrap_or_default())
        }
    }
    impl Drop for CodexHomeGuard {
        fn drop(&mut self) {
            unsafe { std::env::set_var("CODEX_HOME", &self.0) };
        }
    }

    let apply_profile = |home: &std::path::Path, model_list: &str| {
        let model_slug = model_list.lines().next().unwrap_or("").to_string();
        let profile = RelayProfile {
            id: "e2e-compose".to_string(),
            name: "E2E Compose".to_string(),
            model: model_slug.clone(),
            relay_mode: RelayMode::PureApi,
            protocol: RelayProtocol::Responses,
            config_contents: format!(
                r#"model = "{model_slug}"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://relay.example.test/v1"
"#
            ),
            auth_contents: r#"{"OPENAI_API_KEY":"sk-e2e-redacted"}"#.to_string(),
            model_list: model_list.to_string(),
            ..RelayProfile::default()
        };
        apply_relay_profile_to_home_with_switch_rules(home, &profile, "").unwrap();
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let doc: toml_edit::DocumentMut = config.parse().unwrap();
        let catalog_path = doc["model_catalog_json"].as_str().unwrap().to_string();
        let catalog_path = if PathBuf::from(&catalog_path).is_absolute() {
            PathBuf::from(&catalog_path)
        } else {
            home.join(catalog_path)
        };
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&catalog_path).unwrap())
            .unwrap()
    };

    // ── A 段：造冲突缓存，钉死字段级裁决（精调覆盖冲突字段，官方流入未覆盖字段）──
    let temp_a = tempfile::tempdir().unwrap();
    std::fs::write(
        temp_a.path().join("models_cache.json"),
        serde_json::json!({
            "models": [{
                "slug": "gpt-5.6-sol",
                "display_name": "GPT-5.6 Sol Runtime",
                "context_window": 400_000u64,
                "max_context_window": 400_000u64,
                "shell_type": "shell_command",
                "truncation_policy": { "mode": "tokens", "limit": 999 }
            }]
        })
        .to_string(),
    )
    .unwrap();
    let _guard_a = CodexHomeGuard::set(temp_a.path());
    let catalog = apply_profile(temp_a.path(), "gpt-5.6-sol");
    let model = &catalog["models"][0];
    assert_eq!(model["context_window"], 272_000, "精调窗口覆盖官方缓存值");
    assert_eq!(
        model["max_context_window"], 872_000,
        "精调 max 覆盖官方缓存值"
    );
    assert_eq!(model["display_name"], "GPT-5.6-Sol", "精调展示名覆盖缓存值");
    assert_eq!(model["additional_speed_tiers"], serde_json::json!(["fast"]));
    assert_eq!(
        model["truncation_policy"]["limit"], 999,
        "精调未定义的截断策略应从官方缓存流入"
    );
    println!("[E2E] A 段 OK：精调覆盖窗口/展示/Fast 档，官方截断策略(999)流入");
    drop(_guard_a);

    // ── B 段：复制本机真实官方缓存（只读），真实重叠 slug 的组合验证 ──
    let real_cache =
        codex_plus_core::codex_home::default_codex_home_dir().join("models_cache.json");
    let real_cache_text = std::fs::read_to_string(&real_cache).ok();
    let codex = codex_bin();
    let mut composed_home: Option<tempfile::TempDir> = None;
    if let Some(text) = real_cache_text.as_deref() {
        let real: serde_json::Value = serde_json::from_str(text).unwrap();
        let overlap: Vec<&str> = ["gpt-5.6-terra", "gpt-5.6-luna"]
            .into_iter()
            .filter(|slug| {
                real["models"]
                    .as_array()
                    .is_some_and(|models| models.iter().any(|m| m["slug"] == *slug))
            })
            .collect();
        if !overlap.is_empty() {
            let temp_b = tempfile::tempdir().unwrap();
            std::fs::copy(&real_cache, temp_b.path().join("models_cache.json")).unwrap();
            let _guard_b = CodexHomeGuard::set(temp_b.path());
            let catalog = apply_profile(temp_b.path(), "gpt-5.6-terra\ngpt-5.6-luna");
            for slug in &overlap {
                let model = catalog["models"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["slug"] == *slug)
                    .expect(slug);
                let cache_entry = real["models"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["slug"] == *slug)
                    .unwrap();
                assert_eq!(
                    model["max_context_window"], 872_000,
                    "{slug}：精调 max 上限保持"
                );
                // 精调文件未定义的字段（truncation_policy / shell_type 等）必须与真实官方缓存一致
                for key in ["truncation_policy", "shell_type"] {
                    if cache_entry.get(key).is_some() {
                        assert_eq!(
                            model[key], cache_entry[key],
                            "{slug}.{key} 应取真实官方缓存值"
                        );
                    }
                }
            }
            println!(
                "[E2E] B 段 OK：真实官方缓存重叠 slug（{}）组合正确，未覆盖字段与官方一致",
                overlap.join("/")
            );
            composed_home = Some(temp_b);
            drop(_guard_b);
        }
    }

    // ── 真实 codex 解析组合出的 catalog（优先 B 段真实数据版）──
    let target_home = composed_home
        .as_ref()
        .map(|t| t.path().to_path_buf())
        .unwrap_or_else(|| temp_a.path().to_path_buf());
    let (ok, text) = run_codex_debug_models(&codex, &target_home);
    assert!(
        ok,
        "真实 codex 解析组合 catalog 失败：\n{}",
        &text[..text.len().min(4000)]
    );
    assert!(text.contains("gpt-5.6"), "codex 输出应包含 gpt-5.6 系模型");
    println!("[E2E] 真实 codex 解析组合 catalog OK");
}
