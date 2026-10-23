//! 原生 Fast 的本机身份投影；只输出布尔值，不把供应商地址或凭据送入补丁。

use crate::settings::BackendSettings;
use serde::Serialize;
use std::path::Path;
use toml_edit::{DocumentMut, Item};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSettings {
    pub enhancements_enabled: bool,
    pub codex_app_native_fast_unlock: bool,
    pub native_fast_local_api_configured: bool,
}

pub(crate) fn runtime_settings(settings: &BackendSettings) -> RuntimeSettings {
    RuntimeSettings {
        enhancements_enabled: settings.enhancements_enabled,
        codex_app_native_fast_unlock: settings.codex_app_native_fast_unlock,
        native_fast_local_api_configured: local_api_configured_from_home(
            &crate::codex_home::default_codex_home_dir(),
        ),
    }
}

fn local_api_configured_from_home(home: &Path) -> bool {
    std::fs::read_to_string(home.join("config.toml"))
        .ok()
        .is_some_and(|contents| local_api_configured_from_config(&contents))
}

fn local_api_configured_from_config(contents: &str) -> bool {
    let Ok(document) = contents.parse::<DocumentMut>() else {
        return false;
    };
    // 不从管理器的选中档案猜身份：关闭配置同步或外部编辑后，实际生效的配置可能不同。
    let profile = match document.get("profile") {
        None => None,
        Some(item) => {
            let Some(profile) = item.as_str().and_then(|name| {
                document
                    .get("profiles")?
                    .as_table_like()?
                    .get(name)?
                    .as_table_like()
            }) else {
                return false;
            };
            Some(profile)
        }
    };
    let Some(provider_id) = profile
        .and_then(|profile| profile.get("model_provider"))
        .or_else(|| document.get("model_provider"))
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return false;
    };
    // 官方登录与 Bedrock 继续遵循原生身份门控，不能借配置覆盖放行空账号。
    if ["openai", "amazon-bedrock", "amazonBedrock"]
        .iter()
        .any(|reserved| provider_id.eq_ignore_ascii_case(reserved))
    {
        return false;
    }
    let Some(provider) = document
        .get("model_providers")
        .and_then(Item::as_table_like)
        .and_then(|providers| providers.get(provider_id))
        .and_then(Item::as_table_like)
    else {
        return false;
    };
    // 只接受明确声明的 Responses API 连接。配置缺失、无效或身份未知时不放行。
    if provider.get("requires_openai_auth").and_then(Item::as_bool) != Some(false)
        || provider.get("wire_api").and_then(Item::as_str) != Some("responses")
    {
        return false;
    }
    provider
        .get("base_url")
        .and_then(Item::as_str)
        .and_then(|value| reqwest::Url::parse(value.trim()).ok())
        .is_some_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUSTOM: &str = r#"model_provider = "custom"
[model_providers.custom]
wire_api = "responses"
requires_openai_auth = false
base_url = "https://provider.example/v1"
experimental_bearer_token = "private-fixture-token"
"#;

    #[test]
    fn custom_api_without_native_login_is_recognized_without_reading_auth() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), CUSTOM).unwrap();
        assert!(local_api_configured_from_home(dir.path()));
        assert!(!dir.path().join("auth.json").exists());
        // env_key、header 或无认证的本地 API 都不需要把凭据复制给 renderer。
        assert!(local_api_configured_from_config(&CUSTOM.replace(
            "experimental_bearer_token = \"private-fixture-token\"",
            "env_key = \"CUSTOM_API_KEY\""
        )));
    }

    #[test]
    fn missing_invalid_and_unknown_configs_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!local_api_configured_from_home(dir.path()));
        for config in [
            String::new(),
            "not = [valid".to_string(),
            CUSTOM.replace(
                "model_provider = \"custom\"",
                "model_provider = \"unknown\"",
            ),
            CUSTOM.replace("model_provider = \"custom\"", ""),
            CUSTOM.replace(
                "requires_openai_auth = false",
                "requires_openai_auth = true",
            ),
            CUSTOM.replace("requires_openai_auth = false", ""),
            CUSTOM.replace("wire_api = \"responses\"", "wire_api = \"chat\""),
            CUSTOM.replace("wire_api = \"responses\"", ""),
            CUSTOM.replace("https://provider.example/v1", ""),
            CUSTOM.replace("https://provider.example/v1", "not-a-url"),
            CUSTOM.replace("https://provider.example/v1", "file:///tmp/provider"),
            format!("profile = \"missing\"\n{CUSTOM}"),
        ] {
            assert!(!local_api_configured_from_config(&config));
        }
    }

    #[test]
    fn official_identities_cannot_be_reclassified_by_custom_provider_fields() {
        for provider in ["openai", "OpenAI", "amazon-bedrock", "amazonBedrock"] {
            assert!(!local_api_configured_from_config(
                &CUSTOM.replace("custom", provider)
            ));
        }
    }

    #[test]
    fn active_profile_selects_the_effective_provider() {
        let custom_profile = format!(
            "profile = \"api\"\n{}\n[profiles.api]\nmodel_provider = \"custom\"\n",
            CUSTOM.replace("model_provider = \"custom\"", "model_provider = \"openai\"")
        );
        assert!(local_api_configured_from_config(&custom_profile));
        let official_profile = format!(
            "profile = \"official\"\n{CUSTOM}\n[profiles.official]\nmodel_provider = \"openai\"\n"
        );
        assert!(!local_api_configured_from_config(&official_profile));
        assert!(local_api_configured_from_config(&format!(
            "profile = \"inherited\"\n{CUSTOM}\n[profiles.inherited]\nmodel = \"model\"\n"
        )));
    }

    #[test]
    fn runtime_projection_contains_only_flags() {
        let value = serde_json::to_value(RuntimeSettings {
            enhancements_enabled: true,
            codex_app_native_fast_unlock: true,
            native_fast_local_api_configured: local_api_configured_from_config(CUSTOM),
        })
        .unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "enhancementsEnabled": true,
                "codexAppNativeFastUnlock": true,
                "nativeFastLocalApiConfigured": true,
            })
        );
        assert!(!value.to_string().contains("private-fixture-token"));
        assert!(!value.to_string().contains("provider.example"));
    }
}
