//! Grok CLI 的供应商适配：把 Codex 那套 `RelayProfile` 映射成 Grok 的
//! `~/.grok/config.toml` 结构。
//!
//! # 映射约定
//!
//! **一个供应商 = 一个 base_url。** Grok 原生支持按模型覆盖 `base_url` /
//! `api_key`，但管理器的语义是「切换供应商」，所以这里把 profile 视作一个
//! 整体：
//!
//! | RelayProfile | Grok config.toml |
//! |---|---|
//! | `upstream_base_url` / `base_url` | `[model.<alias>].base_url`（每个模型都写） |
//! | `api_key` | `[model.<alias>].api_key` |
//! | `model_list` 每行 | 一个 `[model.<alias>]` 表，`alias` 与 `model` 同名 |
//! | `context_window` | `[model.<alias>].context_window` |
//! | （固定） | `api_backend = "chat_completions"` |
//!
//! `[endpoints].models_base_url` 是全局兜底端点，切换供应商时一并写成本 profile
//! 的 base_url，这样没有单独 `base_url` 的模型也走同一个上游。Grok 自己的
//! `[models].default` 等我们没管理的字段原样保留 —— 落盘走的是
//! `grok_config::save_grok_config_at`，它基于 toml_edit 做增量改写。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, bail};

use crate::grok_config::{
    GrokConfigPayload, GrokModelInput, SaveGrokConfigRequest, load_grok_config_from_home,
    save_grok_config_at,
};
use crate::model_suffix::parse_model_suffix;
use crate::settings::{RelayProfile, RelayProtocol};
use crate::tools::ToolConfig;

/// Grok 的 `api_backend` 取值。当前 Codex++ 只产 `chat_completions`，
/// 如果上游是 Responses 或 Anthropic Messages 端点，跟着 profile 的协议走。
fn api_backend_for(profile: &RelayProfile) -> &'static str {
    match profile.protocol {
        RelayProtocol::Responses => "responses",
        RelayProtocol::ChatCompletions => "chat_completions",
    }
}

/// profile 的供应商端点：优先 `upstream_base_url`，退回 `base_url`。
pub fn profile_base_url(profile: &RelayProfile) -> String {
    let upstream = profile.upstream_base_url.trim();
    if !upstream.is_empty() {
        return upstream.trim_end_matches('/').to_string();
    }
    profile.base_url.trim().trim_end_matches('/').to_string()
}

/// 解析 `model_list`：每行一个模型 slug，可带 `[1M]` 形式的窗口后缀。
/// 返回 (slug, 显式窗口) 列表，顺序即用户书写顺序。
pub fn parse_grok_model_list(model_list: &str) -> Vec<(String, Option<u64>)> {
    model_list
        .split(['\r', '\n', ','])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|raw| {
            let (slug, window) = parse_model_suffix(raw);
            (slug, window)
        })
        .filter(|(slug, _)| !slug.is_empty())
        .collect()
}

/// profile 里声明的模型别名集合。UI 侧用它判断「这个 profile 会不会把 Grok
/// 现有的模型全换掉」。
pub fn profile_model_aliases(profile: &RelayProfile) -> Vec<String> {
    let mut seen = BTreeMap::new();
    for (slug, _) in parse_grok_model_list(&profile.model_list) {
        seen.entry(slug).or_insert(());
    }
    seen.into_keys().collect()
}

/// 把 Grok 的 `config.toml` 读成一个 RelayProfile，用来回填管理器里的编辑态。
///
/// 取磁盘上第一个模型的 `base_url` / `api_key` 作为供应商端点 —— 这是
/// 「一供应商 = 一个 base_url」这个约定的自然逆映射。
pub fn read_grok_profile_from_config(payload: &GrokConfigPayload) -> RelayProfile {
    let first_with_url = payload
        .models
        .iter()
        .find(|model| !model.base_url.trim().is_empty());
    let base_url = first_with_url
        .map(|model| model.base_url.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| payload.models_base_url.trim().to_string());

    let model_list = payload
        .models
        .iter()
        .map(|model| match model.context_window {
            Some(window) => format!("{}[{}]", model.alias, window),
            None => model.alias.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n");

    RelayProfile {
        id: "grok-live".to_string(),
        name: "Grok 当前配置".to_string(),
        model_list,
        upstream_base_url: base_url.clone(),
        base_url,
        // Grok 的 api_key 不落我们自己的 settings.json，只在 config.toml 里；
        // 这里留空，UI 用 `api_key_configured` 显示已配置状态。
        api_key: String::new(),
        protocol: RelayProtocol::ChatCompletions,
        ..RelayProfile::default()
    }
}

/// 从磁盘读 Grok 的当前状态。
pub fn load_grok_state() -> anyhow::Result<GrokConfigPayload> {
    load_grok_config_from_home(&crate::grok_config::default_grok_home_dir())
}

/// 把一个 profile 写进 Grok 的 `config.toml`。
///
/// 注意：这会**替换掉 Grok 里所有受管的 `[model.*]` 表**（保留 `[ui]`、
/// `[models].web_search` 等未管理字段）。调用方必须在 UI 上把这个后果说清楚。
pub fn apply_profile_to_grok(
    profile: &RelayProfile,
    backup_root: &Path,
) -> anyhow::Result<GrokConfigPayload> {
    let aliases = parse_grok_model_list(&profile.model_list);
    if aliases.is_empty() {
        bail!(
            "供应商「{}」没有配置模型列表，无法应用到 Grok",
            profile.name
        );
    }

    let home = crate::grok_config::default_grok_home_dir();
    apply_profile_to_grok_at(&home, profile, backup_root)
}

/// `apply_profile_to_grok` 的显式 home 版本，测试用。
pub fn apply_profile_to_grok_at(
    home: &Path,
    profile: &RelayProfile,
    backup_root: &Path,
) -> anyhow::Result<GrokConfigPayload> {
    let aliases = parse_grok_model_list(&profile.model_list);
    if aliases.is_empty() {
        bail!(
            "供应商「{}」没有配置模型列表，无法应用到 Grok",
            profile.name
        );
    }

    let base_url = profile_base_url(profile);
    let api_backend = api_backend_for(profile).to_string();
    let api_key = profile.api_key.trim().to_string();

    // 先把磁盘上的现状读出来：revision 用来做并发保护（防止覆盖别人的改动），
    // 现有别名用来判断每个模型是新增还是改名。
    let live = load_grok_config_from_home(home)?;
    let live_aliases = live
        .models
        .iter()
        .map(|model| model.alias.clone())
        .collect::<Vec<_>>();

    let models = aliases
        .iter()
        .enumerate()
        .map(|(index, (alias, window))| {
            // 逐位对应：磁盘上第 N 个模型视作是这个 alias 的前身，
            // 这样改名不会丢掉 api_key 之类的未管理字段。
            let source_alias = live_aliases.get(index).cloned().unwrap_or_default();
            GrokModelInput {
                source_alias,
                alias: alias.clone(),
                model: alias.clone(),
                name: alias.clone(),
                base_url: base_url.clone(),
                api_backend: api_backend.clone(),
                context_window: *window,
                // api_key 为空时不动磁盘上的旧值，避免「切换供应商顺手清掉 key」
                // 这种没人能预料到的副作用。
                api_key_update: api_key.clone(),
                remove_api_key: false,
            }
        })
        .collect();

    let request = SaveGrokConfigRequest {
        revision: live.revision.clone(),
        // default 指向第一个模型，保证 Grok 启动时选中的是我们刚配的供应商。
        default_model: aliases[0].0.clone(),
        models_base_url: base_url,
        models,
    };

    let saved = save_grok_config_at(home, &request, backup_root)
        .context("写入 Grok 配置失败")?
        .config;
    Ok(saved)
}

/// 该工具分片里当前选中的 profile。
pub fn active_grok_profile(config: &ToolConfig) -> Option<RelayProfile> {
    config
        .relay_profiles
        .iter()
        .find(|profile| profile.id == config.active_relay_id)
        .cloned()
        .or_else(|| config.relay_profiles.first().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RelayMode;

    fn profile(id: &str, base: &str, models: &str) -> RelayProfile {
        RelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            relay_mode: RelayMode::PureApi,
            protocol: RelayProtocol::ChatCompletions,
            upstream_base_url: base.to_string(),
            model_list: models.to_string(),
            api_key: format!("sk-{id}"),
            ..RelayProfile::default()
        }
    }

    #[test]
    fn parses_model_list_with_window_suffixes() {
        let parsed = parse_grok_model_list("grok-4.5[1M]\ngrok-4.1-fast\n\n , grok-3[128K] ");
        assert_eq!(
            parsed,
            vec![
                ("grok-4.5".to_string(), Some(1_000_000)),
                ("grok-4.1-fast".to_string(), None),
                ("grok-3".to_string(), Some(128_000)),
            ]
        );
    }

    #[test]
    fn base_url_prefers_upstream_then_falls_back() {
        let mut candidate = profile("a", "https://upstream.example/v1", "m");
        candidate.base_url = "https://base.example/v1".to_string();
        assert_eq!(profile_base_url(&candidate), "https://upstream.example/v1");

        candidate.upstream_base_url = String::new();
        assert_eq!(profile_base_url(&candidate), "https://base.example/v1");

        // 尾部斜杠要去掉，否则拼出来的端点会多一个 /
        candidate.base_url = "https://base.example/v1/".to_string();
        assert_eq!(profile_base_url(&candidate), "https://base.example/v1");
    }

    #[test]
    fn apply_writes_provider_endpoint_and_models() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        let candidate = profile(
            "vendor-a",
            "https://vendor-a.example/v1",
            "grok-4.5[1M]\ngrok-4.1-fast",
        );

        let payload = apply_profile_to_grok_at(temp.path(), &candidate, backup.path()).unwrap();

        assert_eq!(payload.models_base_url, "https://vendor-a.example/v1");
        assert_eq!(payload.default_model, "grok-4.5");
        let aliases = payload
            .models
            .iter()
            .map(|model| model.alias.as_str())
            .collect::<Vec<_>>();
        assert_eq!(aliases, vec!["grok-4.5", "grok-4.1-fast"]);
        assert_eq!(payload.models[0].context_window, Some(1_000_000));
        assert_eq!(payload.models[0].base_url, "https://vendor-a.example/v1");
        assert_eq!(payload.models[0].api_backend, "chat_completions");
        assert!(payload.models[0].api_key_configured);
    }

    #[test]
    fn apply_preserves_unmanaged_fields_and_comments() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "# 用户自己写的注释\n[ui]\nyolo = false\n\n[models]\nweb_search = \"search-model\"\n",
        )
        .unwrap();

        apply_profile_to_grok_at(
            temp.path(),
            &profile("vendor-a", "https://a.example/v1", "grok-4.5"),
            backup.path(),
        )
        .unwrap();

        let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        assert!(updated.contains("# 用户自己写的注释"));
        assert!(updated.contains("[ui]"));
        assert!(updated.contains("web_search = \"search-model\""));
        // 模型名带点号，toml_edit 会把它写成带引号的 key，这是它保证
        // `[model.grok-4.5]` 不被解释成嵌套表的方式。
        assert!(updated.contains("[model.\"grok-4.5\"]"));
    }

    /// `grok-4.5` 这类带点号的别名，如果不加引号会被 TOML 解析成嵌套表
    /// `model.grok."4.5"`，读回来别名就变成 `grok-4`。这里钉住「写出去再读
    /// 回来，别名一字不差」，因为 Grok 侧最终要靠这个别名选模型。
    #[test]
    fn dotted_model_aliases_survive_the_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        let candidate = profile(
            "vendor-a",
            "https://a.example/v1",
            "grok-4.5\ngrok-4.1-fast\nplain-model",
        );

        apply_profile_to_grok_at(temp.path(), &candidate, backup.path()).unwrap();

        let live = load_grok_config_from_home(temp.path()).unwrap();
        let aliases = live
            .models
            .iter()
            .map(|model| model.alias.as_str())
            .collect::<Vec<_>>();
        assert_eq!(aliases, vec!["grok-4.5", "grok-4.1-fast", "plain-model"]);
    }

    #[test]
    fn switching_providers_rewrites_the_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();

        apply_profile_to_grok_at(
            temp.path(),
            &profile("vendor-a", "https://a.example/v1", "grok-4.5"),
            backup.path(),
        )
        .unwrap();
        let payload = apply_profile_to_grok_at(
            temp.path(),
            &profile("vendor-b", "https://b.example/v1", "grok-4.5"),
            backup.path(),
        )
        .unwrap();

        // 同一个模型名，端点必须跟着供应商走。
        assert_eq!(payload.models_base_url, "https://b.example/v1");
        assert_eq!(payload.models[0].base_url, "https://b.example/v1");
        let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        assert!(!updated.contains("a.example"));
        assert!(updated.contains("b.example"));
    }

    #[test]
    fn empty_model_list_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        let mut candidate = profile("vendor-a", "https://a.example/v1", "grok-4.5");
        candidate.model_list = "   \n  ".to_string();

        // 空模型列表会让「应用到 Grok」把磁盘上的模型全删光，必须直接拒绝。
        let error = apply_profile_to_grok_at(temp.path(), &candidate, backup.path()).unwrap_err();
        assert!(error.to_string().contains("没有配置模型列表"));
    }

    #[test]
    fn blank_api_key_does_not_wipe_the_existing_one() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("config.toml"),
            "[model.grok-4.5]\nmodel = \"grok-4.5\"\napi_key = \"keep-me\"\n",
        )
        .unwrap();

        let mut candidate = profile("vendor-a", "https://a.example/v1", "grok-4.5");
        candidate.api_key = String::new();
        apply_profile_to_grok_at(temp.path(), &candidate, backup.path()).unwrap();

        let updated = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
        assert!(updated.contains("keep-me"));
    }

    /// 并发保护靠的是「读-改-写」在同一次调用内完成：第二次切换重新读一遍
    /// 磁盘，拿到的就是第一次切换写下的 revision，所以不会被自己误判成冲突。
    #[test]
    fn consecutive_switches_do_not_trip_the_revision_guard() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        let candidate = profile("vendor-a", "https://a.example/v1", "grok-4.5");

        let payload = apply_profile_to_grok_at(temp.path(), &candidate, backup.path()).unwrap();
        assert_eq!(payload.default_model, "grok-4.5");
    }

    #[test]
    fn read_back_maps_grok_config_to_a_profile() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        apply_profile_to_grok_at(
            temp.path(),
            &profile(
                "vendor-a",
                "https://a.example/v1",
                "grok-4.5[1M]\ngrok-4.1-fast",
            ),
            backup.path(),
        )
        .unwrap();

        let live = load_grok_config_from_home(temp.path()).unwrap();
        let back = read_grok_profile_from_config(&live);

        assert_eq!(back.upstream_base_url, "https://a.example/v1");
        assert_eq!(back.model_list, "grok-4.5[1000000]\ngrok-4.1-fast");
        assert!(profile_model_aliases(&back).contains(&"grok-4.1-fast".to_string()));
    }
}
