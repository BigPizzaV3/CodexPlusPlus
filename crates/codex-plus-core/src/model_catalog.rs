use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::settings::{BackendSettings, RelayProfile, SettingsStore};
use serde_json::{Map, Value, json};

const BASE_URL_ENV_KEYS: &[&str] = &[
    "CODEX_PLUS_OPENAI_BASE_URL",
    "CODEX_PLUS_BASE_URL",
    "OPENAI_BASE_URL",
    "OPENAI_API_BASE_URL",
    "OPENAI_API_BASE",
    "OPENAI_API_URL",
];
const API_KEY_ENV_KEYS: &[&str] = &[
    "CODEX_PLUS_OPENAI_API_KEY",
    "CODEX_PLUS_API_KEY",
    "OPENAI_API_KEY",
];

#[derive(Debug, Clone)]
struct ModelSource {
    source_id: String,
    source_type: String,
    name: String,
    base_url: String,
    api_key: String,
    /// 供应商自定义请求头（issue #1685）。非供应商来源为空。
    headers: Vec<crate::settings::RelayHeaderKeyValue>,
    /// Anthropic Messages 协议：模型列表要 `x-api-key` + `anthropic-version`，且默认只返回 20 条。
    anthropic: bool,
}

#[derive(Debug, Default)]
struct CodexConfig {
    root: HashMap<String, String>,
    profiles: HashMap<String, HashMap<String, String>>,
    model_providers: HashMap<String, HashMap<String, String>>,
    effective_model: String,
}

pub async fn read_codex_model_catalog() -> Value {
    let home = codex_home_dir();
    let settings_path = crate::paths::default_settings_path();
    let mut loaded_settings = None;
    if settings_path.exists() {
        if let Ok(settings) = SettingsStore::new(settings_path).load() {
            let profile = settings.active_relay_profile();
            let catalog = if let Some(aggregate) = crate::aggregate_catalog::active_model_aggregation(&settings) {
                crate::aggregate_catalog::catalog_summary(&home, &settings, &aggregate)
                    .unwrap_or_else(|_| relay_profile_model_catalog_value(&home, &profile))
            } else {
                relay_profile_model_catalog_value(&home, &profile)
            };
            if catalog
                .get("models")
                .and_then(Value::as_array)
                .map_or(false, |m| !m.is_empty())
            {
                return model_catalog_with_prices(catalog, Some(&settings), &home);
            }
            loaded_settings = Some(settings);
        }
    }
    let env = std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            Some((name, value.to_string_lossy().into_owned()))
        })
        .collect::<HashMap<_, _>>();
    let client = match crate::http_client::proxied_client("CodexPlusPlus/1.0") {
        Ok(client) => client,
        Err(error) => {
            return model_catalog_with_prices(json!({
                "status": "failed",
                "path": home.join("config.toml").to_string_lossy(),
                "message": error.to_string(),
                "service_tier": config_service_tier_value(&home),
                "model": "",
                "effective_config_model": "",
                "model_provider": "",
                "provider_name": "",
                "default_model": "",
                "models": [],
                "modelMetadata": {},
                "sources": [],
                "responses_api": responses_api_status("unknown", "", "")
            }), loaded_settings.as_ref(), &home);
        }
    };
    let catalog = read_codex_model_catalog_from_home(&home, &env, client).await;
    model_catalog_with_prices(catalog, loaded_settings.as_ref(), &home)
}

/// 给目录里每个模型附上公开价格表的单价（`codex_plus_price`），前端模型选择器按它画价格条。
/// 只读本地缓存的价格表，不阻塞路由；没有缓存时后台预热，本次不带价格。
fn model_catalog_with_prices(mut catalog: Value, settings: Option<&BackendSettings>, home: &Path) -> Value {
    catalog["runtimeContext"] = runtime_context_value(home).unwrap_or_else(|error| {
        json!({ "status": "failed", "message": format!("{error:#}") })
    });
    apply_model_price_overlay(&mut catalog, settings);
    catalog
}

fn runtime_context_value(home: &Path) -> anyhow::Result<Value> {
    let config_path = home.join("config.toml");
    let config_text = match std::fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
        Err(error) => return Err(error).context("读取运行时上下文配置失败"),
    };
    let config: toml::Value = toml::from_str(&config_text).context("解析运行时上下文配置失败")?;
    let Some(pointer) = config.get("model_catalog_json").and_then(toml::Value::as_str) else {
        return Ok(Value::Null);
    };
    if !crate::relay_config::is_codex_plus_managed_model_catalog(home, pointer) {
        return Ok(Value::Null);
    }
    let catalog_path = Path::new(pointer);
    let catalog_path = if catalog_path.is_absolute() {
        catalog_path.to_path_buf()
    } else {
        home.join(catalog_path)
    };
    let catalog_text = std::fs::read_to_string(catalog_path).context("读取受管运行时模型目录失败")?;
    let catalog: Value = serde_json::from_str(&catalog_text).context("解析受管运行时模型目录失败")?;
    let models = catalog.get("models").and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("受管运行时模型目录缺少 models 数组"))?;
    if models.iter().any(|model| model.get("max_context_window").is_some_and(|value| !value.is_null())) {
        return Ok(json!({
            "status": "unsupported",
            "message": "当前受管模型目录仍含启动时窗口上限，无法保证运行中调大上下文窗口"
        }));
    }
    let root_window = config.get("model_context_window")
        .map(|value| value.as_integer().context("model_context_window 必须为整数"))
        .transpose()?;
    let root_compact = config.get("model_auto_compact_token_limit")
        .map(|value| value.as_integer().context("model_auto_compact_token_limit 必须为整数"))
        .transpose()?;
    let mut contexts = Map::new();
    for model in models {
        let slug = model.get("slug").and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("受管运行时模型目录缺少模型 slug"))?;
        let window = root_window.or_else(|| model.get("context_window").and_then(Value::as_i64))
            .filter(|window| *window > 0)
            .ok_or_else(|| anyhow::anyhow!("模型 {slug} 缺少有效上下文窗口"))?;
        let compact = root_compact
            .or_else(|| model.get("auto_compact_token_limit").and_then(Value::as_i64))
            .unwrap_or_else(|| ((window as i128 * 9) / 10) as i64);
        contexts.insert(slug.to_string(), json!({
            "model_context_window": window,
            "model_auto_compact_token_limit": compact
        }));
    }
    Ok(json!({ "status": "ok", "models": contexts }))
}

#[cfg(test)]
mod runtime_context_tests {
    use super::*;

    #[test]
    fn runtime_context_uses_live_managed_catalog_defaults_and_explicit_compaction() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("model-catalogs")).unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "model_catalog_json = 'model-catalogs/live.json'\n",
        ).unwrap();
        for max in [Some(Value::Null), None] {
            let mut model = json!({
                "slug": "model-a",
                "context_window": 131072,
                "auto_compact_token_limit": null,
                "effective_context_window_percent": 95
            });
            if let Some(max) = max {
                model["max_context_window"] = max;
            }
            std::fs::write(
                home.path().join("model-catalogs/live.json"),
                json!({ "models": [model, {
                    "slug": "model-b",
                    "context_window": 262144,
                    "auto_compact_token_limit": 100000
                }] }).to_string(),
            ).unwrap();
            let context = runtime_context_value(home.path()).unwrap();
            assert_eq!(context["status"], "ok");
            assert_eq!(context["models"]["model-a"]["model_context_window"], 131072);
            // 原生 auto compact 基于原始窗口；95% 只影响可用上下文，不重复打折。
            assert_eq!(context["models"]["model-a"]["model_auto_compact_token_limit"], 117964);
            assert_eq!(context["models"]["model-b"]["model_auto_compact_token_limit"], 100000);
        }
    }

    #[test]
    fn runtime_context_preserves_live_root_single_value_overrides() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("model-catalogs")).unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "model_catalog_json = 'model-catalogs/live.json'\nmodel_context_window = 400000\nmodel_auto_compact_token_limit = 300000\n",
        ).unwrap();
        std::fs::write(
            home.path().join("model-catalogs/live.json"),
            r#"{"models":[{"slug":"model-a","context_window":131072,"max_context_window":null,"auto_compact_token_limit":100000}]}"#,
        ).unwrap();
        assert_eq!(runtime_context_value(home.path()).unwrap()["models"]["model-a"], json!({
            "model_context_window": 400000,
            "model_auto_compact_token_limit": 300000
        }));
    }

    #[test]
    fn runtime_context_rejects_legacy_clamp_and_reports_managed_file_errors() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("model-catalogs")).unwrap();
        std::fs::write(
            home.path().join("config.toml"),
            "model_catalog_json = 'model-catalogs/live.json'\n",
        ).unwrap();
        assert_eq!(
            model_catalog_with_prices(json!({}), None, home.path())["runtimeContext"]["status"],
            "failed"
        );
        std::fs::write(home.path().join("model-catalogs/live.json"), "{bad").unwrap();
        assert_eq!(
            model_catalog_with_prices(json!({}), None, home.path())["runtimeContext"]["status"],
            "failed"
        );
        std::fs::write(
            home.path().join("model-catalogs/live.json"),
            r#"{"models":[{"slug":"model-a","context_window":131072,"max_context_window":131072}]}"#,
        ).unwrap();
        assert_eq!(runtime_context_value(home.path()).unwrap()["status"], "unsupported");
        std::fs::write(
            home.path().join("config.toml"),
            "model_catalog_json = 'external.json'\n",
        ).unwrap();
        // 不读取或接管外部目录，即使其内容无效。
        std::fs::write(home.path().join("external.json"), "{bad").unwrap();
        assert!(runtime_context_value(home.path()).unwrap().is_null());
    }
}

fn apply_model_price_overlay(catalog: &mut Value, settings: Option<&BackendSettings>) {
    let catalog_provider = catalog
        .get("model_provider")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let Some(metadata) = catalog
        .get_mut("modelMetadata")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if metadata.is_empty() {
        return;
    }
    let Some(prices) = crate::model_pricing::cached_catalog() else {
        warm_price_catalog();
        return;
    };
    overlay_prices(metadata, &prices, |group_id| {
        group_base_url(settings, &catalog_provider, group_id)
    });
}

/// 模型归属供应商的请求地址：聚合条目用成员的 base_url 查成员自己的价目，
/// 其他（单供应商目录、官方目录）落到目录级 `model_provider` 对应的 profile。
fn group_base_url(
    settings: Option<&BackendSettings>,
    catalog_provider: &str,
    group_id: &str,
) -> String {
    let wanted = if group_id.trim().is_empty() {
        catalog_provider.trim()
    } else {
        group_id.trim()
    };
    if wanted.is_empty() {
        return String::new();
    }
    settings
        .and_then(|settings| {
            settings
                .relay_profiles
                .iter()
                .find(|profile| profile.id == wanted)
        })
        .map(|profile| profile.base_url.trim().to_string())
        .unwrap_or_default()
}

/// 条目对应的上游模型名：聚合成员的 slug 是 `cpx-<成员>/<上游>`，剥掉成员前缀；
/// 默认成员的 slug 本来就是裸名，`[1M]` 这类窗口后缀交给查价的候选名处理。
fn entry_upstream_model<'a>(slug: &'a str, group_id: &str) -> &'a str {
    if group_id.trim().is_empty() {
        return slug;
    }
    let prefix = crate::aggregate_catalog::aggregate_model_id(group_id, "");
    slug.strip_prefix(&prefix).unwrap_or(slug)
}

fn overlay_prices(
    metadata: &mut Map<String, Value>,
    prices: &crate::model_pricing::PriceCatalog,
    base_url_for: impl Fn(&str) -> String,
) {
    for (slug, entry) in metadata.iter_mut() {
        let Some(entry) = entry.as_object_mut() else { continue };
        let group_id = entry
            .get("codex_plus_group_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let upstream = entry_upstream_model(slug, group_id);
        let Some(found) = prices.lookup(&base_url_for(group_id), upstream) else {
            continue;
        };
        entry.insert(
            "codex_plus_price".to_string(),
            json!({
                "input": found.input,
                "cachedInput": found.cached_input,
                "output": found.output,
                "currency": found.currency,
                "provider": found.provider_name,
                "source": "models.dev"
            }),
        );
    }
}

/// 缓存缺失时后台拉一次价格表：本次响应不带价格，下次打开选择器就有了。
fn warm_price_catalog() {
    static WARMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if WARMING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        let _ = crate::model_pricing::load_catalog(false).await;
        WARMING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

fn relay_profile_model_catalog_value(home: &Path, profile: &RelayProfile) -> Value {
    let models = relay_profile_model_ids(profile);
    let model = profile.model.trim().to_string();
    let (live_config, _, _) = load_codex_config(&home.join("config.toml"));
    let codex_model_provider = codex_model_provider_for_relay_profile(home, profile);
    let configured_model = crate::relay_config::relay_profile_model(profile);
    let default_model = if models.iter().any(|item| item == &configured_model) {
        configured_model
    } else {
        models.first().cloned().unwrap_or_default()
    };
    let provider_name = if profile.name.trim().is_empty() {
        profile.id.trim()
    } else {
        profile.name.trim()
    };
    let model_count = models.len();
    let model_metadata = relay_profile_model_ui_metadata_map(profile, &models);
    json!({
        "status": if models.is_empty() { "not_configured" } else { "ok" },
        "path": home.join("config.toml").to_string_lossy(),
        "service_tier": config_service_tier_value(home),
        "model": model,
        "effective_config_model": live_config.effective_model,
        "model_provider": profile.id.trim(),
        "codex_model_provider": codex_model_provider,
        "provider_name": provider_name,
        "default_model": default_model,
        "models": models,
        "modelMetadata": model_metadata,
        "sources": [
            {
                "id": format!("relay-profile:{}", profile.id),
                "type": "relay_profile_model_list",
                "name": provider_name,
                "base_url": profile.base_url.trim(),
                "status": "ok",
                "models": model_count,
                "responses_api": responses_api_status("unknown", "", "")
            }
        ],
        "responses_api": responses_api_status("unknown", "", "")
    })
}

/// 只报告实际配置中显式声明的模型，并考虑当前启用的 profile。
pub(crate) fn effective_config_model(contents: &str) -> String {
    let Ok(config) = contents.parse::<toml::Value>() else {
        return String::new();
    };
    let Some(root) = config.as_table() else {
        return String::new();
    };
    let profile = match root.get("profile") {
        Some(profile) => {
            let Some(profile) = profile.as_str() else {
                return String::new();
            };
            let Some(profile) = root
                .get("profiles")
                .and_then(toml::Value::as_table)
                .and_then(|profiles| profiles.get(profile))
                .and_then(toml::Value::as_table)
            else {
                return String::new();
            };
            Some(profile)
        }
        None => None,
    };
    profile
        .and_then(|profile| profile.get("model"))
        .or_else(|| root.get("model"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

pub fn codex_model_provider_for_relay_profile(home: &Path, profile: &RelayProfile) -> String {
    let profile_config = parse_codex_config(&profile.config_contents);
    let profile_provider = string_value(profile_config.root.get("model_provider"));
    if !profile_provider.is_empty() {
        return profile_provider;
    }

    let (live_config, _, error) = load_codex_config(&home.join("config.toml"));
    if error.is_some() {
        return String::new();
    }
    string_value(live_config.root.get("model_provider"))
}

fn relay_profile_model_ids(profile: &RelayProfile) -> Vec<String> {
    unique_strings(
        profile
            .model_list
            .split(['\r', '\n', ','])
            .chain(std::iter::once(profile.model.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect(),
    )
}

/// 当前活动供应商在 Codex 模型列表里公布的模型 ID 集合，用于清理历史残留引用
/// （例如换目录后模型选择器里还留着的 `ccs-xxx/model`）。
///
/// - 模型聚合：目录里的全部 `cpx-*`——目录由 Codex++ 生成，这份集合是权威的；
///   成员未就绪导致目录算不出来时返回空集，调用方据此跳过清理，绝不误删。
/// - 官方登录且没混用 API Key：模型列表由 Codex 自己决定，返回空集表示「不要清理」。
/// - 普通 API 供应商：它的 model_list + 当前 model + 模型路由的源模型名，
///   并同时收下剥掉 `[1M]` 这类窗口后缀的形式，宁可多留不可误删。
pub fn active_published_model_ids(settings: &crate::settings::BackendSettings) -> HashSet<String> {
    if let Some(aggregate) = crate::aggregate_catalog::active_model_aggregation(settings) {
        // 配了默认供应商时，目录外的模型名仍有兜底转发，清「最近使用」会误删可用条目。
        if crate::aggregate_catalog::default_backs_unbound_models(&aggregate) {
            return HashSet::new();
        }
        return crate::aggregate_catalog::active_model_bindings(settings)
            .map(|bindings| {
                bindings
                    .into_iter()
                    .map(|binding| binding.catalog_model)
                    .collect()
            })
            .unwrap_or_default();
    }
    let profile = settings.active_relay_profile();
    if profile.relay_mode == crate::settings::RelayMode::Official && !profile.official_mix_api_key {
        return HashSet::new();
    }
    let mut ids = HashSet::new();
    for model in relay_profile_model_ids(&profile) {
        let (stripped, _) = crate::model_suffix::parse_model_suffix(&model);
        if !stripped.is_empty() {
            ids.insert(stripped);
        }
        ids.insert(model);
    }
    for route in &profile.model_routes {
        let model = route.model.trim();
        if !model.is_empty() {
            ids.insert(model.to_string());
        }
    }
    ids
}

fn model_ui_metadata_map(models: &[String]) -> Value {
    model_ui_metadata_map_with_overrides(models, "")
}

fn model_ui_metadata_map_with_overrides(models: &[String], user_json: &str) -> Value {
    let user: Value = serde_json::from_str(user_json).unwrap_or(Value::Null);
    let mut metadata = Map::new();
    for model in models {
        let slug = crate::model_suffix::parse_model_suffix(model).0;
        let user_override = user.as_object().and_then(|entries| entries.iter()
            .find(|(key, value)| value.is_object() && crate::model_suffix::parse_model_suffix(key).0.eq_ignore_ascii_case(&slug))
            .map(|(_, value)| value));
        if let Some(value) = crate::model_suffix::model_ui_metadata_with_override(model, user_override) {
            metadata.insert(model.clone(), value);
        }
    }
    Value::Object(metadata)
}

fn relay_profile_model_ui_metadata_map(profile: &RelayProfile, models: &[String]) -> Value {
    let mut metadata = model_ui_metadata_map_with_overrides(models, &profile.model_metadata)
        .as_object().cloned().unwrap_or_default();
    let windows: std::collections::BTreeMap<String, String> =
        serde_json::from_str::<Value>(&profile.model_windows)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .map(|object| object.into_iter()
                .filter_map(|(key, value)| value.as_str().map(|text| (key, text.to_string())))
                .collect())
            .unwrap_or_default();
    let fallback = crate::model_suffix::parse_window_token(&profile.context_window);
    for model in models {
        let manual = crate::model_suffix::lookup_model_map(&windows, model)
            .and_then(|token| crate::model_suffix::parse_window_token(token));
        if let Some(window) = manual {
            // 逐模型手工窗口优先级最高，覆盖内置模板。
            let entry = metadata.entry(model.clone()).or_insert_with(|| json!({}));
            for field in ["contextWindow", "maxContextWindow", "context_window", "max_context_window"] {
                entry[field] = json!(window);
            }
            continue;
        }
        // 没有手工窗口时，供应商级「上下文大小」只作兜底：模型模板里已经声明了
        // 自己的窗口就别拿全局值盖掉它。否则多模型列表 + 全局窗口会把每个模型都
        // 压成同一个数，模型自身的真实窗口反而丢失（issue #2590）。
        let Some(fallback) = fallback else { continue };
        let entry = metadata.entry(model.clone()).or_insert_with(|| json!({}));
        for field in ["contextWindow", "maxContextWindow", "context_window", "max_context_window"] {
            let missing = entry
                .get(field)
                .and_then(Value::as_u64)
                .is_none_or(|existing| existing == 0);
            if missing {
                entry[field] = json!(fallback);
            }
        }
    }
    Value::Object(metadata)
}

#[cfg(test)]
mod window_override_tests {
    use super::*;

    #[test]
    fn manual_window_precedes_profile_and_builtin_metadata() {
        let models = vec!["gpt-5.5".to_string(), "vendor-model".to_string()];
        let profile = RelayProfile {
            model_windows: r#"{"GPT-5.5[1M]":"1M"}"#.into(),
            context_window: "128K".into(),
            ..Default::default()
        };
        let metadata = relay_profile_model_ui_metadata_map(&profile, &models);
        for field in ["contextWindow", "maxContextWindow", "context_window", "max_context_window"] {
            assert_eq!(metadata["gpt-5.5"][field], 1_000_000);
            assert_eq!(metadata["vendor-model"][field], 128_000);
        }

        // issue #2590：全局窗口只兜底，不能盖掉模型模板自带的窗口。
        // 同一份列表里 gpt-5.5 有内置模板、vendor-model 没有，前者应保持模板值。
        let builtin_only = relay_profile_model_ui_metadata_map(&RelayProfile::default(), &models);
        let builtin_window = builtin_only["gpt-5.5"]["contextWindow"]
            .as_u64()
            .expect("gpt-5.5 应有内置窗口");
        assert_ne!(builtin_window, 128_000, "前置条件：内置窗口不应恰好等于全局值");
        assert_eq!(metadata["gpt-5.5"]["contextWindow"], 1_000_000);
        let global_only = RelayProfile {
            context_window: "128K".into(),
            ..Default::default()
        };
        let with_global = relay_profile_model_ui_metadata_map(&global_only, &models);
        assert_eq!(
            with_global["gpt-5.5"]["contextWindow"], builtin_window,
            "全局窗口不应压过内置模板"
        );
        assert_eq!(with_global["vendor-model"]["contextWindow"], 128_000);

        let metadata = relay_profile_model_ui_metadata_map(&RelayProfile::default(), &models);
        assert!(metadata["gpt-5.5"]["contextWindow"].as_u64().is_some());
        assert!(metadata.get("vendor-model").is_none());
    }
}

pub async fn read_codex_model_catalog_from_home(
    home: &Path,
    env: &HashMap<String, String>,
    client: reqwest::Client,
) -> Value {
    let config_path = home.join("config.toml");
    let auth_api_key = read_codex_auth_api_key(&home.join("auth.json"));
    let (config, effective, error) = load_codex_config(&config_path);
    let mut model = string_value(effective.get("model"));
    let mut model_provider = string_value(effective.get("model_provider"));
    let (resolved_provider, provider_config) =
        provider_config_for_model_provider(&config, &model_provider);
    if model_provider.is_empty() && !resolved_provider.is_empty() {
        model_provider = resolved_provider;
    }
    let provider_name = provider_config
        .as_ref()
        .and_then(|provider| provider.get("name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| model_provider.clone());

    if let Some(error) = error.as_ref().filter(|error| *error != "missing") {
        return json!({
            "status": "failed",
            "path": config_path.to_string_lossy(),
            "message": error,
            "service_tier": service_tier_value(&effective),
            "model": model,
            "effective_config_model": config.effective_model,
            "model_provider": model_provider,
            "provider_name": provider_name,
            "default_model": "",
            "models": [],
            "modelMetadata": {},
            "sources": [],
            "responses_api": responses_api_status("unknown", "", "")
        });
    }

    let mut sources = model_sources_from_environment(env, &auth_api_key);
    if error.is_none() {
        if let Some(source) = model_source_from_config(&config, &effective, env, &auth_api_key) {
            if sources
                .iter()
                .all(|existing| trim_url(&existing.base_url) != trim_url(&source.base_url))
            {
                sources.push(source);
            }
        }
    }

    let mut source_statuses = Vec::new();
    let mut models = Vec::new();
    for source in sources.iter() {
        let (source_models, mut source_status) = fetch_models_from_source(&client, source).await;
        source_status["responses_api"] = responses_api_status("unknown", "", "");
        models.extend(source_models);
        source_statuses.push(source_status);
    }
    let (catalog_models, catalog_metadata, catalog_status) =
        models_from_config_model_catalog_json(home, &effective);
    models.extend(catalog_models);
    if let Some(status) = catalog_status {
        source_statuses.push(status);
    }

    models = unique_strings(models);
    if model.is_empty() {
        model = string_value(effective.get("default_model"));
    }
    let default_model = if models.iter().any(|item| item == &model) {
        model.clone()
    } else {
        models.first().cloned().unwrap_or_default()
    };
    let status = if !models.is_empty() {
        "ok"
    } else if !source_statuses.is_empty()
        && source_statuses
            .iter()
            .any(|source| source.get("status").and_then(Value::as_str) == Some("failed"))
    {
        "failed"
    } else if error.as_deref() == Some("missing") {
        "missing"
    } else {
        "not_configured"
    };
    let responses_api = preferred_responses_api_status(&source_statuses);
    let mut model_metadata = model_ui_metadata_map(&models);
    for (model, fields) in catalog_metadata {
        model_metadata[model] = fields;
    }

    json!({
        "status": status,
        "path": config_path.to_string_lossy(),
        "service_tier": service_tier_value(&effective),
        "model": model,
        "effective_config_model": config.effective_model,
        "model_provider": model_provider,
        "provider_name": provider_name,
        "default_model": default_model,
        "models": models,
        "modelMetadata": model_metadata,
        "sources": source_statuses,
        "responses_api": responses_api
    })
}

fn codex_home_dir() -> PathBuf {
    crate::codex_home::default_codex_home_dir()
}

// 读取 config.toml（含 profile 覆盖）里生效的 service_tier；未配置时返回 null。
fn service_tier_value(effective: &HashMap<String, String>) -> Value {
    let tier = string_value(effective.get("service_tier"));
    if tier.is_empty() {
        Value::Null
    } else {
        Value::String(tier)
    }
}

fn config_service_tier_value(home: &Path) -> Value {
    let (_, effective, _) = load_codex_config(&home.join("config.toml"));
    service_tier_value(&effective)
}

fn load_codex_config(path: &Path) -> (CodexConfig, HashMap<String, String>, Option<String>) {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (
                CodexConfig::default(),
                HashMap::new(),
                Some("missing".to_string()),
            );
        }
        Err(error) => {
            return (
                CodexConfig::default(),
                HashMap::new(),
                Some(error.to_string()),
            );
        }
    };
    let config = parse_codex_config(&contents);
    let mut effective = config.root.clone();
    if let Some(profile) = config.root.get("profile") {
        if let Some(profile_values) = config.profiles.get(profile) {
            effective.extend(profile_values.clone());
        }
    }
    (config, effective, None)
}

fn parse_codex_config(contents: &str) -> CodexConfig {
    let mut config = CodexConfig {
        effective_model: effective_config_model(contents),
        ..CodexConfig::default()
    };
    let mut section = ConfigSection::Root;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = ConfigSection::from_header(trimmed.trim_matches(&['[', ']'][..]));
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = unquote_toml_string(value);
        match &section {
            ConfigSection::Root => {
                config.root.insert(key, value);
            }
            ConfigSection::Profile(name) => {
                config
                    .profiles
                    .entry(name.clone())
                    .or_default()
                    .insert(key, value);
            }
            ConfigSection::ModelProvider(name) => {
                config
                    .model_providers
                    .entry(name.clone())
                    .or_default()
                    .insert(key, value);
            }
            ConfigSection::Other => {}
        }
    }
    config
}

#[derive(Debug, Clone)]
enum ConfigSection {
    Root,
    Profile(String),
    ModelProvider(String),
    Other,
}

impl ConfigSection {
    fn from_header(header: &str) -> Self {
        if let Some(name) = header.strip_prefix("profiles.") {
            return Self::Profile(name.trim_matches('"').to_string());
        }
        if let Some(name) = header.strip_prefix("model_providers.") {
            return Self::ModelProvider(name.trim_matches('"').to_string());
        }
        Self::Other
    }
}

fn read_codex_auth_api_key(path: &Path) -> String {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let Ok(payload) = serde_json::from_str::<Value>(&contents) else {
        return String::new();
    };
    for key in [
        "OPENAI_API_KEY",
        "api_key",
        "apikey",
        "access_token",
        "token",
    ] {
        let value = payload
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }
    String::new()
}

fn provider_config_for_model_provider(
    config: &CodexConfig,
    model_provider: &str,
) -> (String, Option<HashMap<String, String>>) {
    if !model_provider.is_empty() {
        return (
            model_provider.to_string(),
            config.model_providers.get(model_provider).cloned(),
        );
    }
    if config.model_providers.len() == 1 {
        if let Some((name, provider)) = config.model_providers.iter().next() {
            return (name.clone(), Some(provider.clone()));
        }
    }
    (model_provider.to_string(), None)
}

fn model_sources_from_environment(
    env: &HashMap<String, String>,
    auth_api_key: &str,
) -> Vec<ModelSource> {
    let base_url = first_env_value(env, BASE_URL_ENV_KEYS);
    if base_url.is_empty() {
        return Vec::new();
    }
    let api_key = first_env_value(env, API_KEY_ENV_KEYS);
    vec![ModelSource {
        source_id: "env:openai-compatible".to_string(),
        source_type: "environment".to_string(),
        name: "Environment".to_string(),
        base_url,
        api_key: if api_key.is_empty() {
            auth_api_key.to_string()
        } else {
            api_key
        },
        headers: Vec::new(),
        anthropic: false,
    }]
}

fn model_source_from_config(
    config: &CodexConfig,
    effective: &HashMap<String, String>,
    env: &HashMap<String, String>,
    auth_api_key: &str,
) -> Option<ModelSource> {
    let model_provider = string_value(effective.get("model_provider"));
    let (resolved_provider, provider_config) =
        provider_config_for_model_provider(config, &model_provider);
    let provider_config = provider_config?;
    let base_url = string_value(provider_config.get("base_url"));
    if base_url.is_empty() {
        return None;
    }
    let name = string_value(provider_config.get("name"));
    let api_key = provider_api_key(&provider_config, env, auth_api_key);
    Some(ModelSource {
        source_id: format!(
            "config:{}",
            if resolved_provider.is_empty() {
                &name
            } else {
                &resolved_provider
            }
        ),
        source_type: "config".to_string(),
        name: if name.is_empty() {
            resolved_provider
        } else {
            name
        },
        base_url,
        api_key,
        headers: Vec::new(),
        anthropic: false,
    })
}

fn provider_api_key(
    provider_config: &HashMap<String, String>,
    env: &HashMap<String, String>,
    auth_api_key: &str,
) -> String {
    for key in [
        "experimental_bearer_token",
        "api_key",
        "apikey",
        "bearer_token",
        "token",
    ] {
        let value = string_value(provider_config.get(key));
        if !value.is_empty() {
            return value;
        }
    }
    for key in [
        "env_key",
        "api_key_env",
        "api_key_env_var",
        "key_env",
        "bearer_token_env",
    ] {
        let env_name = string_value(provider_config.get(key));
        if !env_name.is_empty() {
            let value = first_env_value(env, &[&env_name]);
            if !value.is_empty() {
                return value;
            }
        }
    }
    let env_key = first_env_value(env, API_KEY_ENV_KEYS);
    if env_key.is_empty() {
        auth_api_key.to_string()
    } else {
        env_key
    }
}

/// 单个模型列表请求的整体预算（发请求 + 读响应体）。模型列表是小体量非流式响应，
/// 30 秒足够；此前请求没有任何超时，上游不响应时管理器的「获取模型」会永久挂起。
const MODELS_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

async fn fetch_models_from_source(
    client: &reqwest::Client,
    source: &ModelSource,
) -> (Vec<String>, Value) {
    fetch_models_from_source_with_timeout(client, source, MODELS_FETCH_TIMEOUT).await
}

async fn fetch_models_from_source_with_timeout(
    client: &reqwest::Client,
    source: &ModelSource,
    timeout: std::time::Duration,
) -> (Vec<String>, Value) {
    let (models, status, _) = fetch_models_payload_from_source(client, source, timeout).await;
    (models, status)
}

/// 拉一次模型列表，除了模型名和状态，还带回解析好的响应体（拿到模型时才有），
/// 供「从上游同步上下文窗口 / 思考等级」读模型元数据（#2807）。请求、超时、错误判定与
/// [`fetch_models_from_source_with_timeout`] 完全相同，不另发请求。
async fn fetch_models_payload_from_source(
    client: &reqwest::Client,
    source: &ModelSource,
    timeout: std::time::Duration,
) -> (Vec<String>, Value, Option<Value>) {
    let mut endpoint = models_endpoint(&source.base_url);
    if source.anthropic && !endpoint.is_empty() {
        // Anthropic 的 /v1/models 默认一页 20 条，一次取满（上限 1000）。
        endpoint.push(if endpoint.contains('?') { '&' } else { '?' });
        endpoint.push_str("limit=1000");
    }
    let mut safe_source = json!({
        "id": source.source_id,
        "type": source.source_type,
        "name": source.name,
        "base_url": safe_url_for_status(&source.base_url),
        "endpoint": safe_url_for_status(&endpoint),
        "auth": if source.api_key.is_empty() { "missing" } else { "present" },
    });
    if endpoint.is_empty() {
        safe_source["status"] = json!("failed");
        safe_source["message"] = json!("Missing base URL");
        safe_source["models"] = json!(0);
        return (Vec::new(), safe_source, None);
    }

    let mut request = client
        .get(&endpoint)
        .header(reqwest::header::ACCEPT, "application/json");
    // 与协议代理、供应商测试共用 relay_headers：显式 Authorization 优先于 API Key。
    let has_header = |name: &str| source.headers.iter().any(|header| header.key.trim().eq_ignore_ascii_case(name));
    let official_anthropic = source.anthropic && endpoint.contains("anthropic.com");
    if !source.api_key.is_empty() && !crate::relay_headers::has_authorization(&source.headers) && !official_anthropic {
        request = request.bearer_auth(&source.api_key);
    }
    if source.anthropic {
        // 官方只认 x-api-key；第三方中转常常两种都收，Bearer 已在上面带上。
        if !source.api_key.is_empty() && !has_header("x-api-key") {
            request = request.header("x-api-key", &source.api_key);
        }
        if !has_header("anthropic-version") {
            request = request.header("anthropic-version", crate::anthropic_bridge::ANTHROPIC_VERSION);
        }
    }
    request = crate::relay_headers::apply_headers(request, &source.headers);

    match tokio::time::timeout(timeout, async move {
        let response = request.send().await?;
        let status = response.status().as_u16();
        let body = response.text().await?;
        Ok::<_, reqwest::Error>((status, body))
    })
    .await
    {
        Err(_elapsed) => {
            let (models, status) = failed_source(
                safe_source,
                format!(
                    "上游 {} 秒内未返回模型列表（已超时中断）",
                    timeout.as_secs()
                ),
            );
            (models, status, None)
        }
        Ok(Err(error)) => {
            let (models, status) = failed_source(safe_source, error.to_string());
            (models, status, None)
        }
        Ok(Ok((status_code, body))) => {
            let (models, status) = interpret_models_response(status_code, &body, safe_source);
            let payload = if models.is_empty() {
                None
            } else {
                serde_json::from_str::<Value>(body.trim()).ok()
            };
            (models, status, payload)
        }
    }
}

fn interpret_models_response(
    status_code: u16,
    body: &str,
    mut safe_source: Value,
) -> (Vec<String>, Value) {
    let payload = serde_json::from_str::<Value>(body.trim()).ok();
    if !(200..300).contains(&status_code) {
        return failed_source(
            safe_source,
            format!("HTTP {status_code}{}", upstream_error_detail(body)),
        );
    }
    let Some(payload) = payload else {
        return failed_source(
            safe_source,
            format!(
                "HTTP {status_code} 响应不是有效 JSON{}",
                upstream_error_detail(body)
            ),
        );
    };
    // 业务错误信封优先于模型列表判定：网关在错误态下可能回一个占位或部分
    // 列表（issue 复现：{"code":401,"success":false,"data":[{"id":"..."}]}），
    // 先看列表会把它当成「获取成功」，用户看到成功但实际令牌已失效。
    // 反过来不会误伤正常响应——business_error_message 只认 success=false /
    // code∉{0,200} / error 非空，标准 OpenAI 响应三者都没有。
    if let Some(message) = business_error_message(&payload) {
        return failed_source(safe_source, message);
    }
    let models = unique_strings(parse_model_payload(&payload));
    if !models.is_empty() {
        safe_source["status"] = json!("ok");
        safe_source["models"] = json!(models.len());
        return (models, safe_source);
    }
    // HTTP 200 且无业务错误信封：维持既有语义，按“网关可达但 0 个模型”处理
    safe_source["status"] = json!("ok");
    safe_source["models"] = json!(0);
    (Vec::new(), safe_source)
}

fn failed_source(mut source: Value, message: String) -> (Vec<String>, Value) {
    source["status"] = json!("failed");
    source["message"] = json!(message);
    source["models"] = json!(0);
    source["responses_api"] = responses_api_status("unknown", "", "");
    (Vec::new(), source)
}

/// 部分网关用 HTTP 200 + 业务信封承载失败。智谱 Codex 专属端点（/api/v1）在 key
/// 缺失或无效时返回 `{"code":401,"msg":"令牌已过期或验证不正确","success":false}`，
/// HTTP 状态仍是 200，此前只认状态码，这类失败被解析成“0 个模型”，报错完全不
/// 指向真因（#2190）。只在模型列表解析为空时才判信封——能取到数据就优先信数据，
/// 避免误伤非标成功响应。
fn business_error_message(payload: &Value) -> Option<String> {
    let object = payload.as_object()?;
    let envelope_failed = match object.get("success").and_then(Value::as_bool) {
        Some(success) => !success,
        None => {
            numeric_business_code(object.get("code")).is_some_and(|failed| failed)
                || object.get("error").is_some_and(|error| !error.is_null())
        }
    };
    if !envelope_failed {
        return None;
    }
    message_from_payload(object)
}

/// `code` 字段是否落在失败档：数字（含字符串数字）且不等于 0/200 视为失败，
/// 0 是常见的“成功”码，200 是 OpenAI 系成功码。非数字 code 不参与判定。
fn numeric_business_code(code: Option<&Value>) -> Option<bool> {
    let code = code?;
    let code = code.as_i64().or_else(|| {
        code.as_str()
            .and_then(|code| code.trim().parse::<i64>().ok())
    })?;
    Some(code != 0 && code != 200)
}

/// 从错误体对象里提取人话原因，兼容 OpenAI 系 `{"error":{"message":...}}`、
/// 智谱 `{"msg":...}`、xAI `{"error":"..."}` 等形态。
fn message_from_payload(object: &Map<String, Value>) -> Option<String> {
    for key in ["error", "msg", "message"] {
        let Some(value) = object.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        let text = match value {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Object(nested) => nested
                .get("message")
                .and_then(Value::as_str)
                .map(|text| text.trim().to_string()),
            _ => None,
        };
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            return Some(text);
        }
    }
    None
}

/// 非 2xx（或非 JSON）时把上游错误体里的原因带进报错（如
/// `HTTP 401：Incorrect API key provided`）。此前只报 "HTTP 401"，key 失效、
/// 地址填错、套餐未开通全靠用户猜。
fn upstream_error_detail(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let detail = serde_json::from_str::<Value>(trimmed)
        .ok()
        .and_then(|payload| message_from_payload(payload.as_object()?))
        .unwrap_or_else(|| truncate_for_message(trimmed));
    format!("：{detail}")
}

/// 报错信息里附带的响应体上限，避免上游回一整页 HTML 时把 UI 提示撑爆。
fn truncate_for_message(text: &str) -> String {
    const MAX_CHARS: usize = 200;
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= MAX_CHARS {
        return text;
    }
    let mut truncated: String = text.chars().take(MAX_CHARS).collect();
    truncated.push('…');
    truncated
}

fn responses_api_status(status: &str, endpoint: &str, message: &str) -> Value {
    json!({
        "status": status,
        "endpoint": endpoint,
        "message": message
    })
}

pub async fn fetch_relay_profile_model_ids(
    profile: &RelayProfile,
) -> anyhow::Result<(Vec<String>, String)> {
    // 官方登录 profile 没有 Base URL：带账号 OAuth 凭据读官方模型目录，
    // 「从上游获取」按钮对官方账号同样可用（issue 讨论场景：聚合成员要填模型列表）。
    if crate::official_login::member_uses_official_login_transport(profile) {
        return fetch_official_login_model_ids(profile).await;
    }
    let source = relay_profile_model_source(profile)?;
    let endpoint = models_endpoint(&source.base_url);
    let client = crate::http_client::proxied_client_for_relay(&profile.user_agent, &profile)?;
    let (models, status) = fetch_models_from_source(&client, &source).await;
    if models.is_empty() {
        let message = status
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("上游没有返回可用模型");
        anyhow::bail!("{message}");
    }
    Ok((models, endpoint))
}

/// 拉供应商的 `/models`，返回整个响应体（模型名之外还有上游给的上下文窗口、思考等级等元数据，#2807）。
/// 请求方式（代理、超时、请求头、Anthropic 分页）与 [`fetch_relay_profile_model_ids`] 相同；
/// 官方登录供应商走的是官方目录，不在这里处理。没拿到模型时报错。
pub async fn fetch_relay_profile_models_payload(profile: &RelayProfile) -> anyhow::Result<Value> {
    if crate::official_login::member_uses_official_login_transport(profile) {
        anyhow::bail!("官方登录供应商的模型目录由官方提供，不需要从上游同步");
    }
    let source = relay_profile_model_source(profile)?;
    let client = crate::http_client::proxied_client_for_relay(&profile.user_agent, profile)?;
    let (models, status, payload) =
        fetch_models_payload_from_source(&client, &source, MODELS_FETCH_TIMEOUT).await;
    match payload {
        Some(payload) if !models.is_empty() => Ok(payload),
        _ => {
            let message = status
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("上游没有返回可用模型");
            anyhow::bail!("{message}");
        }
    }
}

/// 供应商拉模型列表用的来源（地址、Key、自定义请求头、协议）。官方登录供应商缺凭据、地址为空时报错。
fn relay_profile_model_source(profile: &RelayProfile) -> anyhow::Result<ModelSource> {
    if crate::official_login::is_official_login_profile(profile)
        && profile.base_url.trim().is_empty()
    {
        anyhow::bail!(
            "官方登录供应商「{}」缺少 ChatGPT 登录凭据，请先导入或登录官方账号",
            profile.name
        );
    }
    let source = ModelSource {
        source_id: format!("relay-profile:{}", profile.id),
        source_type: "relay_profile".to_string(),
        name: if profile.name.trim().is_empty() {
            profile.id.clone()
        } else {
            profile.name.trim().to_string()
        },
        base_url: if profile.upstream_base_url.trim().is_empty() {
            profile.base_url.trim().to_string()
        } else {
            profile.upstream_base_url.trim().to_string()
        },
        api_key: crate::relay_config::relay_profile_api_key(profile),
        headers: profile.custom_headers.clone(),
        anthropic: profile.protocol == crate::settings::RelayProtocol::AnthropicMessages,
    };
    if source.base_url.is_empty() {
        anyhow::bail!("Base URL 不能为空");
    }
    Ok(source)
}

/// 官方模型目录的客户端版本参数；与 Codex CLI 的取值保持一致。
pub(crate) const OFFICIAL_MODELS_CLIENT_VERSION: &str = "0.159.0";

/// 官方登录 profile 的模型列表：带账号 OAuth 凭据请求 chatgpt.com 官方目录。
async fn fetch_official_login_model_ids(
    profile: &RelayProfile,
) -> anyhow::Result<(Vec<String>, String)> {
    let credential = crate::official_login::resolve_official_credential(profile, false).await?;
    let base = crate::official_login::official_base_url();
    let endpoint = format!("{}/models", base.trim().trim_end_matches('/'));
    let version = crate::gateway::chatgpt_backend::codex_version(
        &crate::codex_home::default_codex_home_dir(),
    );
    let client = crate::http_client::proxied_client_for_relay(&profile.user_agent, &profile)?;
    let mut request = client
        .get(&endpoint)
        .query(&[("client_version", version.as_str())])
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", credential.access_token.trim()),
        )
        .header("originator", "codex_cli_rs")
        .header("version", &version)
        .header(
            reqwest::header::USER_AGENT,
            crate::gateway::chatgpt_backend::codex_user_agent(&version),
        )
        .timeout(MODELS_FETCH_TIMEOUT);
    if !credential.account_id.trim().is_empty() {
        request = request.header("chatgpt-account-id", credential.account_id.trim());
    }
    let response = request.send().await.context("官方模型列表请求失败")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "获取官方模型列表失败：HTTP {status} {}",
            truncate_for_message(&text)
        );
    }
    let value: Value = serde_json::from_str(&text).context("官方模型列表解析失败")?;
    let models = parse_official_model_ids(&value);
    if models.is_empty() {
        anyhow::bail!("官方账号没有返回可见模型");
    }
    Ok((models, endpoint))
}

/// 官方目录条目：slug/id/name 均可作为模型 id；`visibility=hide` 与
/// `visible_in_picker=false` 是官方内部预留位，不进入供应商模型列表。
/// 例外是 `codex-auto-review`：留在列表里才能在网关里选它当审批模型；
/// 写进 Codex 目录时仍标成隐藏（见 `aggregate_catalog::catalog_visibility`）。
pub(crate) fn parse_official_model_ids(value: &Value) -> Vec<String> {
    let entries = value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.get("models").and_then(Value::as_array))
        .or_else(|| value.get("items").and_then(Value::as_array))
        .or_else(|| value.as_array());
    let Some(entries) = entries else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let (id, hidden) = match entry {
            Value::String(id) => (id.trim().to_string(), false),
            Value::Object(map) => {
                let hidden = map
                    .get("visibility")
                    .and_then(Value::as_str)
                    .is_some_and(|visibility| visibility.eq_ignore_ascii_case("hide"))
                    || map.get("visible_in_picker").and_then(Value::as_bool) == Some(false);
                let id = ["slug", "id", "name"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str))
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                (id, hidden)
            }
            _ => (String::new(), false),
        };
        let hidden =
            hidden && !id.eq_ignore_ascii_case(crate::aggregate_catalog::CODEX_AUTO_REVIEW_MODEL);
        if id.is_empty() || hidden || !seen.insert(id.clone()) {
            continue;
        }
        ids.push(id);
    }
    ids
}

fn preferred_responses_api_status(sources: &[Value]) -> Value {
    let statuses = sources
        .iter()
        .filter_map(|source| source.get("responses_api"))
        .collect::<Vec<_>>();
    for wanted in ["unsupported", "supported", "failed"] {
        if let Some(status) = statuses
            .iter()
            .find(|status| status.get("status").and_then(Value::as_str) == Some(wanted))
        {
            return (*status).clone();
        }
    }
    responses_api_status("unknown", "", "")
}

fn models_endpoint(base_url: &str) -> String {
    let cleaned = safe_url_for_status(base_url).trim_end_matches('/').to_string();
    if cleaned.is_empty() {
        return String::new();
    }
    // 与实际代理共用规则，保留 /v1beta/openai 等已有路径；清理后的 URL 不含凭据。
    crate::protocol_proxy::models_url(&cleaned)
}

fn parse_model_payload(payload: &Value) -> Vec<String> {
    if let Some(array) = payload.as_array() {
        return array
            .iter()
            .filter_map(|item| {
                item.as_str().map(str::to_string).or_else(|| {
                    item.as_object().and_then(|object| {
                        ["id", "model", "name"]
                            .iter()
                            .filter_map(|key| {
                                object
                                    .get(*key)
                                    .and_then(Value::as_str)
                                    .map(|value| model_id_from_field(key, value))
                                    .filter(|value| !value.is_empty())
                            })
                            .next()
                            .map(ToString::to_string)
                    })
                })
            })
            .collect();
    }
    let Some(object) = payload.as_object() else {
        return Vec::new();
    };
    for key in ["data", "models", "items"] {
        if let Some(value) = object.get(key) {
            let nested = parse_model_payload(value);
            if !nested.is_empty() {
                return nested;
            }
        }
    }
    ["id", "model", "name"]
        .iter()
        .filter_map(|key| {
            object
                .get(*key)
                .and_then(Value::as_str)
                .map(|value| model_id_from_field(key, value))
                .filter(|value| !value.is_empty())
        })
        .next()
        .map(|value| vec![value.to_string()])
        .unwrap_or_default()
}

/// 模型 ID 字段取值。Gemini 的列表把 ID 放在 `name` 字段且带 `models/` 前缀
/// （如 `models/gemini-2.5-pro`），剥掉前缀让 ID 与请求体里的 `model` 值一致。
fn model_id_from_field<'a>(key: &str, value: &'a str) -> &'a str {
    let value = value.trim();
    if key == "name" {
        value.strip_prefix("models/").unwrap_or(value)
    } else {
        value
    }
}

fn models_from_config_model_catalog_json(
    home: &Path,
    effective: &HashMap<String, String>,
) -> (Vec<String>, Map<String, Value>, Option<Value>) {
    let raw_path = string_value(effective.get("model_catalog_json"));
    if raw_path.is_empty() {
        return (Vec::new(), Map::new(), None);
    }
    let path = resolve_config_path(home, &raw_path);
    let safe_path = path.to_string_lossy().to_string();
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) => {
            return (
                Vec::new(),
                Map::new(),
                Some(json!({
                    "id": "config:model_catalog_json",
                    "type": "model_catalog_json",
                    "name": "Codex model catalog",
                    "path": safe_path,
                    "status": "failed",
                    "message": error.to_string(),
                    "models": 0,
                    "responses_api": responses_api_status("unknown", "", "")
                })),
            );
        }
    };
    let payload = match serde_json::from_str::<Value>(&contents) {
        Ok(payload) => payload,
        Err(error) => {
            return (
                Vec::new(),
                Map::new(),
                Some(json!({
                    "id": "config:model_catalog_json",
                    "type": "model_catalog_json",
                    "name": "Codex model catalog",
                    "path": safe_path,
                    "status": "failed",
                    "message": error.to_string(),
                    "models": 0,
                    "responses_api": responses_api_status("unknown", "", "")
                })),
            );
        }
    };
    let models = unique_strings(parse_model_catalog_json_models(&payload));
    let metadata = payload["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| catalog_model_visible_in_api(entry))
        .filter_map(|entry| {
            let slug = entry.get("slug")?.as_str()?.trim();
            if slug.is_empty() {
                return None;
            }
            crate::model_suffix::model_ui_metadata_with_override(slug, Some(entry))
                .map(|fields| (slug.to_string(), fields))
        })
        .collect();
    let count = models.len();
    (
        models,
        metadata,
        Some(json!({
            "id": "config:model_catalog_json",
            "type": "model_catalog_json",
            "name": "Codex model catalog",
            "path": safe_path,
            "status": "ok",
            "models": count,
            "responses_api": responses_api_status("unknown", "", "")
        })),
    )
}

fn resolve_config_path(home: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        home.join(path)
    }
}

fn parse_model_catalog_json_models(payload: &Value) -> Vec<String> {
    let Some(models) = payload.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    models
        .iter()
        .filter(|model| catalog_model_visible_in_api(model))
        .filter_map(|model| model.get("slug").and_then(Value::as_str))
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
        .map(str::to_string)
        .collect()
}

fn catalog_model_visible_in_api(model: &Value) -> bool {
    let supported_in_api = model
        .get("supported_in_api")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !supported_in_api {
        return false;
    }
    let visibility = model
        .get("visibility")
        .and_then(Value::as_str)
        .unwrap_or("list")
        .trim();
    visibility.eq_ignore_ascii_case("list")
}

fn unique_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        result.push(value.to_string());
    }
    result
}

fn first_env_value(env: &HashMap<String, String>, names: &[&str]) -> String {
    names
        .iter()
        .filter_map(|name| env.get(*name))
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn safe_url_for_status(url: &str) -> String {
    let mut cleaned = url
        .split('?')
        .next()
        .unwrap_or_default()
        .split('#')
        .next()
        .unwrap_or_default()
        .to_string();
    if let Ok(parsed) = reqwest::Url::parse(&cleaned) {
        let host = parsed.host_str().unwrap_or_default();
        let authority = parsed
            .port()
            .map(|port| format!("{host}:{port}"))
            .unwrap_or_else(|| host.to_string());
        cleaned = format!("{}://{}{}", parsed.scheme(), authority, parsed.path());
    }
    cleaned
}

fn trim_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn string_value(value: Option<&String>) -> String {
    value
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn unquote_toml_string(value: &str) -> String {
    let value = value.trim();
    if let Ok(parsed) = toml::from_str::<toml::Value>(&format!("value = {value}")) {
        if let Some(value) = parsed.get("value").and_then(toml::Value::as_str) {
            return value.to_string();
        }
    }
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
        .to_string()
}

#[cfg(test)]
mod model_fetch_tests {
    use super::*;

    #[test]
    fn relay_profile_model_catalog_keeps_configured_default_after_model_list_sorting() {
        let profile = RelayProfile {
            model: "qwen3-coder".into(),
            model_list: "deepseek-coder\nqwen3-coder\nclaude-compatible".into(),
            config_contents: "model = \"qwen3-coder\"\n".into(),
            ..RelayProfile::default()
        };
        let home = tempfile::tempdir().unwrap();
        let catalog = relay_profile_model_catalog_value(home.path(), &profile);

        assert_eq!(catalog["default_model"], "qwen3-coder");
    }

    #[test]
    fn effective_config_model_requires_valid_explicit_config_and_profile() {
        for (config, expected) in [
            ("", ""),
            ("default_model = 'fallback'\n", ""),
            ("model = ' root '\n", "root"),
            (
                "model = 'root'\nprofile = 'work'\n[profiles.work]\nmodel = 'profile'\n",
                "profile",
            ),
            (
                "model = 'root'\nprofile = 'work'\n[profiles.work]\nother = true\n",
                "root",
            ),
            ("model = 'root'\nprofile = 'missing'\n", ""),
            ("model = 'root'\nprofile = 5\n", ""),
            (
                "model = 'root'\nprofile = 'work'\n[profiles.work]\nmodel = 5\n",
                "",
            ),
            ("model = 'root'\nmodel = 'duplicate'\n", ""),
            ("model = [invalid\n", ""),
        ] {
            assert_eq!(effective_config_model(config), expected, "{config}");
        }
    }

    #[test]
    fn relay_profile_catalog_default_identity_comes_from_live_config() {
        let home = tempfile::tempdir().unwrap();
        let profile = RelayProfile {
            model: "candidate".into(),
            model_list: "candidate\nother".into(),
            config_contents: "model = 'snapshot'\n".into(),
            ..RelayProfile::default()
        };
        let catalog = relay_profile_model_catalog_value(home.path(), &profile);
        assert_eq!(catalog["default_model"], "candidate");
        assert_eq!(catalog["effective_config_model"], "");
        std::fs::write(
            home.path().join("config.toml"),
            "model = 'root'\nprofile = 'work'\n[profiles.work]\nmodel = 'live'\n",
        )
        .unwrap();
        let catalog = relay_profile_model_catalog_value(home.path(), &profile);
        assert_eq!(catalog["effective_config_model"], "live");
        assert!(!catalog["models"].as_array().unwrap().contains(&json!("live")));
    }

    #[test]
    fn model_list_endpoint_preserves_custom_prefixes_and_removes_credentials() {
        for (base, expected) in [
            ("https://example.test/v1beta/openai/", "https://example.test/v1beta/openai/models"),
            ("https://example.test", "https://example.test/v1/models"),
            ("https://example.test/api/coding/v3", "https://example.test/api/coding/v3/models"),
            ("https://user:secret@example.test/v1/models?token=secret", "https://example.test/v1/models"),
        ] { assert_eq!(models_endpoint(base), expected); }
    }

    #[test]
    fn relay_model_priority_declaration_reaches_ui_without_enabling_other_models() {
        let profile = RelayProfile {
            model: "grok-custom".into(), model_list: "grok-custom\ngrok-unknown\ngpt-5.4".into(),
            model_metadata: json!({
                "GROK-CUSTOM[1M]": {"service_tiers":[{"id":"flex"}], "additional_speed_tiers":["slow"], "codex_plus_fast_support":"supported"},
                "gpt-5.4": {"codex_plus_fast_support":"unsupported"}
            }).to_string(),
            ..RelayProfile::default()
        };
        let home = tempfile::tempdir().unwrap();
        let catalog = relay_profile_model_catalog_value(home.path(), &profile);
        let metadata = &catalog["modelMetadata"];
        assert_eq!(metadata["grok-custom"]["prioritySupportOverride"], true);
        assert_eq!(metadata["gpt-5.4"]["prioritySupportOverride"], false);
        assert_eq!(metadata["grok-custom"]["serviceTiers"][0]["id"], "flex");
        assert_eq!(metadata["grok-custom"]["serviceTiers"][1]["id"], "priority");
        assert!(metadata.get("grok-unknown").is_none());
        assert!(metadata["grok-custom"].get("codex_plus_fast_support").is_none());
    }

    #[test]
    fn business_error_message_detects_zhipu_style_envelope() {
        // 智谱 /api/v1 Codex 专属端点在 key 缺失或无效时返回 HTTP 200 + 业务信封（#2190）
        let payload = json!({"code": 401, "msg": "令牌已过期或验证不正确", "success": false});
        assert_eq!(
            business_error_message(&payload).as_deref(),
            Some("令牌已过期或验证不正确")
        );
    }

    #[test]
    fn business_error_message_detects_failed_code_without_success_field() {
        let numeric = json!({"code": 1001, "msg": "鉴权失败"});
        assert_eq!(
            business_error_message(&numeric).as_deref(),
            Some("鉴权失败")
        );

        let string_code = json!({"code": "401", "message": "invalid token"});
        assert_eq!(
            business_error_message(&string_code).as_deref(),
            Some("invalid token")
        );

        let error_object = json!({"error": {"code": "1001", "message": "未收到 Authorization"}});
        assert_eq!(
            business_error_message(&error_object).as_deref(),
            Some("未收到 Authorization")
        );
    }

    #[test]
    fn business_error_message_ignores_success_envelopes() {
        for payload in [
            json!({"code": 200, "success": true, "data": []}),
            json!({"code": 0, "msg": "ok", "data": []}),
            json!({"object": "list", "data": []}),
            json!({"code": "200", "data": []}),
            json!([{"id": "glm-5.3"}]),
        ] {
            assert_eq!(business_error_message(&payload), None, "{payload}");
        }
    }

    #[test]
    fn interpret_models_response_keeps_ok_semantics_when_200_has_no_models() {
        let (models, status) =
            interpret_models_response(200, r#"{"object":"list","data":[]}"#, json!({"id": "t"}));
        assert!(models.is_empty());
        assert_eq!(status["status"], "ok");
        assert_eq!(status["models"], 0);
    }

    #[test]
    fn interpret_models_response_surfaces_business_error_on_http_200() {
        let (models, status) = interpret_models_response(
            200,
            r#"{"code":401,"msg":"令牌已过期或验证不正确","success":false}"#,
            json!({"id": "t"}),
        );
        assert!(models.is_empty());
        assert_eq!(status["status"], "failed");
        assert_eq!(status["message"], "令牌已过期或验证不正确");
    }

    #[test]
    fn interpret_models_response_prefers_business_error_over_a_returned_model_list() {
        // 网关在错误态下回了一个列表时，错误必须优先——否则用户看到「获取成功」
        // 但令牌其实已失效，排查会往错误方向走。
        let (models, status) = interpret_models_response(
            200,
            r#"{"code":401,"msg":"令牌已过期","success":false,"data":[{"id":"glm-5.3"}]}"#,
            json!({"id": "t"}),
        );
        assert!(models.is_empty(), "错误态下的列表不应被当作可用模型");
        assert_eq!(status["status"], "failed");
        assert_eq!(status["message"], "令牌已过期");
        assert_eq!(status["models"], 0);
    }

    #[test]
    fn interpret_models_response_keeps_normal_lists_working_without_an_envelope() {
        // 反向保护：标准 OpenAI 响应没有 success/code/error 字段，
        // 提前检查业务错误不能把它误判成失败。
        for body in [
            r#"{"object":"list","data":[{"id":"gpt-5.6-sol"},{"id":"gpt-5.6-terra"}]}"#,
            r#"{"code":200,"success":true,"data":[{"id":"glm-5.3"}]}"#,
            r#"{"code":0,"msg":"ok","data":[{"id":"deepseek-v4"}]}"#,
        ] {
            let (models, status) = interpret_models_response(200, body, json!({"id": "t"}));
            assert!(!models.is_empty(), "{body}");
            assert_eq!(status["status"], "ok", "{body}");
        }
    }

    #[test]
    fn interpret_models_response_appends_upstream_reason_on_http_error() {
        let (_, status) = interpret_models_response(
            401,
            r#"{"error":{"code":"1001","message":"Header中未收到Authorization参数，无法进行身份验证。"}}"#,
            json!({"id": "t"}),
        );
        assert_eq!(status["status"], "failed");
        let message = status["message"].as_str().unwrap();
        assert!(message.starts_with("HTTP 401"), "{message}");
        assert!(
            message.contains("Header中未收到Authorization参数"),
            "{message}"
        );
    }

    #[test]
    fn interpret_models_response_reports_invalid_json_body() {
        let (_, status) = interpret_models_response(200, "<html>502</html>", json!({"id": "t"}));
        assert_eq!(status["status"], "failed");
        let message = status["message"].as_str().unwrap();
        assert!(message.contains("不是有效 JSON"), "{message}");
        assert!(message.contains("<html>502</html>"), "{message}");
    }

    #[test]
    fn upstream_error_detail_truncates_plain_text_bodies() {
        assert_eq!(
            upstream_error_detail("Authentication Fails (governor)"),
            "：Authentication Fails (governor)"
        );
        assert_eq!(upstream_error_detail("   "), "");
        let long_body = "x".repeat(500);
        let detail = upstream_error_detail(&long_body);
        assert!(detail.chars().count() < 260, "{}", detail.chars().count());
    }

    #[test]
    fn parse_model_payload_strips_gemini_models_prefix() {
        let models = parse_model_payload(&json!({
            "models": [
                {"name": "models/gemini-2.5-pro", "supportedGenerationMethods": ["generateContent"]},
                {"name": "models/gemini-2.5-flash"}
            ]
        }));
        assert_eq!(models, vec!["gemini-2.5-pro", "gemini-2.5-flash"]);
    }

    #[test]
    fn parse_model_payload_keeps_plain_name_fields() {
        let models = parse_model_payload(&json!({
            "models": [{"name": "moonshot-v1"}, {"id": "kimi-k2.6"}]
        }));
        assert_eq!(models, vec!["moonshot-v1", "kimi-k2.6"]);
    }

    #[tokio::test]
    async fn models_fetch_times_out_when_upstream_never_responds() {
        // 只 bind 不 accept：TCP 握手由内核完成，send 能成功，读响应体会一直挂起
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let source = ModelSource {
            source_id: "test".to_string(),
            source_type: "test".to_string(),
            name: "Test".to_string(),
            base_url: format!("http://{address}"),
            api_key: "key".to_string(),
            headers: Vec::new(),
            anthropic: false,
        };
        let (_, status) = fetch_models_from_source_with_timeout(
            &client,
            &source,
            std::time::Duration::from_millis(300),
        )
        .await;
        assert_eq!(status["status"], "failed");
        assert!(
            status["message"]
                .as_str()
                .unwrap()
                .contains("未返回模型列表"),
            "{}",
            status["message"]
        );
        drop(listener);
    }

    #[test]
    fn official_model_ids_skip_hidden_entries_and_dedupe() {
        let value = json!({
            "data": [
                {"slug": "gpt-6.1-sol"},
                {"slug": "gpt-reserve", "visibility": "hide"},
                {"id": "gpt-6-sol"},
                {"name": "gpt-6-luna", "visible_in_picker": false},
                {"slug": "codex-auto-review", "visibility": "hide"},
                "gpt-6.1-sol",
                {"slug": "   "}
            ]
        });
        // 审批模型虽然隐藏，也要留下，网关才能把它选作审批模型。
        assert_eq!(
            parse_official_model_ids(&value),
            vec!["gpt-6.1-sol", "gpt-6-sol", "codex-auto-review"]
        );
    }

    #[test]
    fn official_model_ids_accept_models_and_items_shapes() {
        assert_eq!(
            parse_official_model_ids(&json!({"models": [{"slug": "a"}]})),
            vec!["a"]
        );
        assert_eq!(
            parse_official_model_ids(&json!({"items": ["b"]})),
            vec!["b"]
        );
        assert!(parse_official_model_ids(&json!({"other": []})).is_empty());
    }

    /// 官方登录 profile 的模型拉取：带 OAuth 凭据请求官方 models 端点，跳过隐藏位。
    struct OfficialBaseUrlEnvGuard {
        previous: Option<std::ffi::OsString>,
    }

    impl OfficialBaseUrlEnvGuard {
        fn set(value: &str) -> Self {
            let previous = std::env::var_os("CODEX_PLUS_OFFICIAL_BASE_URL");
            unsafe {
                std::env::set_var("CODEX_PLUS_OFFICIAL_BASE_URL", value);
            }
            Self { previous }
        }
    }

    impl Drop for OfficialBaseUrlEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.previous {
                    Some(value) => std::env::set_var("CODEX_PLUS_OFFICIAL_BASE_URL", value),
                    None => std::env::remove_var("CODEX_PLUS_OFFICIAL_BASE_URL"),
                }
            }
        }
    }

    #[tokio::test]
    async fn official_login_profile_fetches_models_with_oauth_headers() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = r#"{"data":[{"slug":"gpt-6.1-sol"},{"slug":"gpt-reserve","visibility":"hide"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            request
        });
        let _guard = OfficialBaseUrlEnvGuard::set(&format!("http://{address}/backend-api/codex"));

        let profile = RelayProfile {
            id: "official".to_string(),
            name: "官方登录".to_string(),
            relay_mode: crate::settings::RelayMode::Official,
            auth_contents: r#"{"auth_mode":"chatgpt","tokens":{"access_token":"official-access","refresh_token":"r","account_id":"acct-1"}}"#.to_string(),
            ..RelayProfile::default()
        };
        let (models, endpoint) = fetch_relay_profile_model_ids(&profile).await.unwrap();
        assert_eq!(models, vec!["gpt-6.1-sol"]);
        assert!(endpoint.ends_with("/backend-api/codex/models"), "{endpoint}");

        let request = server.await.unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(
            request.starts_with("GET /backend-api/codex/models?client_version="),
            "{request}"
        );
        assert!(
            lower.contains("authorization: bearer official-access"),
            "{request}"
        );
        assert!(lower.contains("chatgpt-account-id: acct-1"), "{request}");
        assert!(lower.contains("originator: codex_cli_rs"), "{request}");
    }
}

#[cfg(test)]
mod price_overlay_tests {
    use super::*;

    const PRICES: &str = r#"{
      "deepseek": { "id": "deepseek", "name": "DeepSeek", "api": "https://api.deepseek.com",
        "models": { "deepseek-chat": { "id": "deepseek-chat", "cost": { "input": 0.27, "output": 1.1, "cache_read": 0.07 } } } },
      "anthropic": { "id": "anthropic", "name": "Anthropic",
        "models": { "claude-sonnet-4-5": { "id": "claude-sonnet-4-5", "cost": { "input": 3, "output": 15 } } } },
      "zhipuai": { "id": "zhipuai", "name": "Zhipu AI", "api": "https://open.bigmodel.cn/api/paas/v4",
        "models": { "glm-4.6": { "id": "glm-4.6", "cost": { "input": 0.6, "output": 2.2 } } } }
    }"#;

    fn prices() -> crate::model_pricing::PriceCatalog {
        crate::model_pricing::PriceCatalog::parse(PRICES).unwrap()
    }

    fn settings() -> BackendSettings {
        let mut settings = BackendSettings::default();
        settings.relay_profiles = vec![
            RelayProfile {
                id: "deepseek".to_string(),
                base_url: "https://api.deepseek.com/v1".to_string(),
                ..RelayProfile::default()
            },
            RelayProfile {
                id: "glm".to_string(),
                base_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
                ..RelayProfile::default()
            },
        ];
        settings
    }

    #[test]
    fn aggregate_entries_lookup_with_member_base_url_and_bare_upstream() {
        let settings = settings();
        let mut catalog = json!({
            "model_provider": "agg",
            "modelMetadata": {
                "cpx-deepseek/deepseek-chat": { "codex_plus_group_id": "deepseek" },
                "cpx-glm/glm-4.6": { "codex_plus_group_id": "glm" },
                "deepseek-chat": { "codex_plus_group_id": "deepseek" },
                "cpx-glm/claude-sonnet-4-5": { "codex_plus_group_id": "glm" },
                "unknown-model": { "codex_plus_group_id": "deepseek" },
                "no-group": {}
            }
        });
        let catalog_provider = catalog["model_provider"].as_str().unwrap().to_string();
        let metadata = catalog["modelMetadata"].as_object_mut().unwrap();
        overlay_prices(metadata, &prices(), |group_id| {
            group_base_url(Some(&settings), &catalog_provider, group_id)
        });
        let entry = &catalog["modelMetadata"]["cpx-deepseek/deepseek-chat"]["codex_plus_price"];
        assert_eq!((entry["input"].as_f64(), entry["output"].as_f64()), (Some(0.27), Some(1.1)));
        assert_eq!(entry["provider"].as_str(), Some("DeepSeek"));
        // 默认成员的裸名 slug 一样能查到，上游名不会被成员前缀卡住。
        assert!(catalog["modelMetadata"]["deepseek-chat"]["codex_plus_price"].is_object());
        // 成员前缀剥掉后按供应商地址查：glm 的 claude 走 zhipu 价目查不到就退回原厂提示价。
        let resold = &catalog["modelMetadata"]["cpx-glm/claude-sonnet-4-5"]["codex_plus_price"];
        assert_eq!(resold["provider"].as_str(), Some("Anthropic"));
        assert!(catalog["modelMetadata"]["unknown-model"].get("codex_plus_price").is_none());
        // 没有分组归属的条目按目录级 model_provider 找 profile，找不到就按名猜。
        assert!(catalog["modelMetadata"]["no-group"].get("codex_plus_price").is_none());
    }

    #[test]
    fn entry_upstream_strips_only_member_prefix() {
        let group = "deepseek";
        assert_eq!(
            entry_upstream_model("cpx-deepseek/anthropic/claude-sonnet-4-5", group),
            "anthropic/claude-sonnet-4-5"
        );
        assert_eq!(entry_upstream_model("deepseek-chat", group), "deepseek-chat");
        assert_eq!(entry_upstream_model("cpx-other/x", ""), "cpx-other/x");
    }
}
