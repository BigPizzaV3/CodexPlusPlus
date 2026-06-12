use std::process::Command;

use anyhow::Context;
use serde_json::Value;
use toml_edit::{DocumentMut, Item};

use crate::settings::{BackendSettings, RelayProfile};

pub const KEYCHAIN_REF_PREFIX: &str = "jiyi-keychain:";
const KEYCHAIN_SERVICE: &str = "com.jiyi.codex.apimart";
const GLOBAL_RELAY_ACCOUNT: &str = "relay:global";
const IDENTITY_SYNC_ACCOUNT: &str = "identity-sync:global";
const LOCAL_BACKEND_SESSION_ACCOUNT: &str = "local-backend-session:active";
const TENCENT_SMS_SECRET_ID_ACCOUNT: &str = "tencent-sms:secret-id";
const TENCENT_SMS_SECRET_KEY_ACCOUNT: &str = "tencent-sms:secret-key";

pub fn global_relay_api_key_account() -> &'static str {
    GLOBAL_RELAY_ACCOUNT
}

pub fn identity_sync_api_key_account() -> &'static str {
    IDENTITY_SYNC_ACCOUNT
}

pub fn local_backend_session_token_account() -> &'static str {
    LOCAL_BACKEND_SESSION_ACCOUNT
}

pub fn tencent_sms_secret_id_account() -> &'static str {
    TENCENT_SMS_SECRET_ID_ACCOUNT
}

pub fn tencent_sms_secret_key_account() -> &'static str {
    TENCENT_SMS_SECRET_KEY_ACCOUNT
}

pub fn relay_profile_api_key_account(profile_id: &str) -> String {
    let normalized = profile_id
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if normalized.is_empty() {
        "relay-profile:default".to_string()
    } else {
        format!("relay-profile:{normalized}")
    }
}

pub fn keychain_ref(account: &str) -> String {
    format!("{KEYCHAIN_REF_PREFIX}{account}")
}

pub fn is_keychain_ref(value: &str) -> bool {
    value.trim().starts_with(KEYCHAIN_REF_PREFIX)
}

pub fn keychain_account_from_ref(value: &str) -> Option<String> {
    value
        .trim()
        .strip_prefix(KEYCHAIN_REF_PREFIX)
        .map(str::trim)
        .filter(|account| !account.is_empty())
        .map(ToString::to_string)
}

pub fn resolve_secret_value(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let Some(account) = keychain_account_from_ref(value) else {
        return value.to_string();
    };
    read_secret(&account).ok().flatten().unwrap_or_default()
}

pub fn protect_local_backend_session_token(token: &str) -> anyhow::Result<String> {
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("本地后端 session token 不能为空。");
    }
    store_plain_secret(local_backend_session_token_account(), token)
}

pub fn resolve_local_backend_session_token() -> String {
    resolve_secret_value(&keychain_ref(local_backend_session_token_account()))
}

pub fn clear_local_backend_session_token() -> anyhow::Result<()> {
    delete_secret(local_backend_session_token_account())
}

pub fn protect_tencent_sms_secret_id(secret_id: &str) -> anyhow::Result<String> {
    let secret_id = secret_id.trim();
    if secret_id.is_empty() {
        anyhow::bail!("腾讯云短信 SecretId 不能为空。");
    }
    store_plain_secret(tencent_sms_secret_id_account(), secret_id)
}

pub fn protect_tencent_sms_secret_key(secret_key: &str) -> anyhow::Result<String> {
    let secret_key = secret_key.trim();
    if secret_key.is_empty() {
        anyhow::bail!("腾讯云短信 SecretKey 不能为空。");
    }
    store_plain_secret(tencent_sms_secret_key_account(), secret_key)
}

pub fn protect_settings_secrets(settings: &mut BackendSettings) -> anyhow::Result<bool> {
    let mut changed = false;
    let global_plain = plaintext_secret(settings.relay_api_key.as_str());
    let global_ref = if let Some(secret) = global_plain.as_deref() {
        let reference = store_plain_secret(global_relay_api_key_account(), secret)?;
        settings.relay_api_key = reference.clone();
        changed = true;
        Some((secret.to_string(), reference))
    } else {
        None
    };
    if let Some(secret) = plaintext_secret(settings.jiyi_identity_sync_api_key.as_str()) {
        settings.jiyi_identity_sync_api_key =
            store_plain_secret(identity_sync_api_key_account(), &secret)?;
        changed = true;
    }

    for profile in &mut settings.relay_profiles {
        let api_key = plaintext_secret(profile.api_key.as_str());
        let auth_key = plaintext_secret(
            auth_openai_api_key(&profile.auth_contents)
                .as_deref()
                .unwrap_or_default(),
        );
        let config_key = plaintext_secret(
            config_experimental_bearer_token(&profile.config_contents)
                .ok()
                .flatten()
                .as_deref()
                .unwrap_or_default(),
        );
        let existing_ref = [
            profile.api_key.as_str(),
            auth_openai_api_key(&profile.auth_contents)
                .as_deref()
                .unwrap_or_default(),
            config_experimental_bearer_token(&profile.config_contents)
                .ok()
                .flatten()
                .as_deref()
                .unwrap_or_default(),
        ]
        .into_iter()
        .find(|value| is_keychain_ref(value))
        .map(str::trim)
        .map(ToString::to_string);
        let plain = api_key
            .as_deref()
            .or(auth_key.as_deref())
            .or(config_key.as_deref());
        let reference = match (plain, existing_ref) {
            (Some(secret), _) => match &global_ref {
                Some((global_secret, reference)) if global_secret == secret => reference.clone(),
                _ => store_plain_secret(&relay_profile_api_key_account(&profile.id), secret)?,
            },
            (None, Some(reference)) => reference,
            (None, None) => continue,
        };

        if !profile.api_key.trim().is_empty() && profile.api_key.trim() != reference {
            profile.api_key = reference.clone();
            changed = true;
        }
        if replace_auth_openai_api_key(&mut profile.auth_contents, &reference)? {
            changed = true;
        }
        if replace_config_experimental_bearer_token(&mut profile.config_contents, &reference)? {
            changed = true;
        }
    }

    Ok(changed)
}

pub fn materialize_relay_profile_secrets(
    profile: &mut RelayProfile,
    fallback_api_key: &str,
) -> anyhow::Result<bool> {
    let mut changed = false;
    let fallback_api_key = fallback_api_key.trim();
    let resolved_api_key = first_non_empty(&[
        resolve_secret_value(&profile.api_key),
        auth_openai_api_key(&profile.auth_contents)
            .map(|value| resolve_secret_value(&value))
            .unwrap_or_default(),
        config_experimental_bearer_token(&profile.config_contents)
            .ok()
            .flatten()
            .map(|value| resolve_secret_value(&value))
            .unwrap_or_default(),
        fallback_api_key.to_string(),
    ]);
    if resolved_api_key.is_empty() {
        return Ok(false);
    }

    if is_keychain_ref(&profile.api_key) {
        profile.api_key = resolved_api_key.clone();
        changed = true;
    }
    if auth_openai_api_key(&profile.auth_contents)
        .as_deref()
        .is_some_and(is_keychain_ref)
        && replace_auth_openai_api_key(&mut profile.auth_contents, &resolved_api_key)?
    {
        changed = true;
    }
    if config_experimental_bearer_token(&profile.config_contents)
        .ok()
        .flatten()
        .as_deref()
        .is_some_and(is_keychain_ref)
        && replace_config_experimental_bearer_token(
            &mut profile.config_contents,
            &resolved_api_key,
        )?
    {
        changed = true;
    }
    Ok(changed)
}

pub fn settings_contain_plaintext_api_key(settings: &BackendSettings) -> bool {
    plaintext_secret(settings.relay_api_key.as_str()).is_some()
        || plaintext_secret(settings.jiyi_identity_sync_api_key.as_str()).is_some()
        || settings.relay_profiles.iter().any(|profile| {
            plaintext_secret(profile.api_key.as_str()).is_some()
                || auth_openai_api_key(&profile.auth_contents)
                    .as_deref()
                    .and_then(plaintext_secret)
                    .is_some()
                || config_experimental_bearer_token(&profile.config_contents)
                    .ok()
                    .flatten()
                    .as_deref()
                    .and_then(plaintext_secret)
                    .is_some()
        })
}

pub fn settings_contain_keychain_ref(settings: &BackendSettings) -> bool {
    is_keychain_ref(&settings.relay_api_key)
        || is_keychain_ref(&settings.jiyi_identity_sync_api_key)
        || settings.relay_profiles.iter().any(|profile| {
            is_keychain_ref(&profile.api_key)
                || auth_openai_api_key(&profile.auth_contents)
                    .as_deref()
                    .is_some_and(is_keychain_ref)
                || config_experimental_bearer_token(&profile.config_contents)
                    .ok()
                    .flatten()
                    .as_deref()
                    .is_some_and(is_keychain_ref)
        })
}

fn plaintext_secret(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || is_keychain_ref(value) {
        return None;
    }
    Some(value.to_string())
}

fn store_plain_secret(account: &str, secret: &str) -> anyhow::Result<String> {
    write_secret(account, secret)?;
    Ok(keychain_ref(account))
}

fn first_non_empty(values: &[String]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn auth_openai_api_key(auth_contents: &str) -> Option<String> {
    let auth: Value = serde_json::from_str(auth_contents).ok()?;
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn replace_auth_openai_api_key(
    auth_contents: &mut String,
    replacement: &str,
) -> anyhow::Result<bool> {
    if auth_openai_api_key(auth_contents).is_none() {
        return Ok(false);
    }
    let mut value =
        serde_json::from_str::<Value>(auth_contents).with_context(|| "auth.json JSON 解析失败")?;
    let Some(object) = value.as_object_mut() else {
        anyhow::bail!("auth.json 必须是 JSON 对象");
    };
    object.insert(
        "OPENAI_API_KEY".to_string(),
        Value::String(replacement.to_string()),
    );
    let updated = format!("{}\n", serde_json::to_string_pretty(&value)?);
    if *auth_contents == updated {
        Ok(false)
    } else {
        *auth_contents = updated;
        Ok(true)
    }
}

fn config_experimental_bearer_token(config_contents: &str) -> anyhow::Result<Option<String>> {
    let doc = config_contents.parse::<DocumentMut>()?;
    let provider_id = doc
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("custom");
    let Some(token) = doc
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(provider_id))
        .and_then(Item::as_table)
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    Ok(Some(token.to_string()))
}

fn replace_config_experimental_bearer_token(
    config_contents: &mut String,
    replacement: &str,
) -> anyhow::Result<bool> {
    if config_experimental_bearer_token(config_contents)?.is_none() {
        return Ok(false);
    }
    let mut doc = config_contents.parse::<DocumentMut>()?;
    let provider_id = doc
        .get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("custom")
        .to_string();
    let Some(provider) = doc
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
        .and_then(|providers| providers.get_mut(&provider_id))
        .and_then(Item::as_table_like_mut)
    else {
        return Ok(false);
    };
    provider.insert("experimental_bearer_token", toml_edit::value(replacement));
    let updated = ensure_trailing_newline(doc.to_string());
    if *config_contents == updated {
        Ok(false)
    } else {
        *config_contents = updated;
        Ok(true)
    }
}

fn ensure_trailing_newline(value: String) -> String {
    if value.ends_with('\n') {
        value
    } else {
        format!("{value}\n")
    }
}

#[cfg(target_os = "macos")]
fn write_secret(account: &str, secret: &str) -> anyhow::Result<()> {
    let output = Command::new("/usr/bin/security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
            "-w",
            secret,
        ])
        .output()
        .with_context(|| "无法调用 macOS 钥匙串 security 命令")?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "写入 macOS 钥匙串失败：{}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[cfg(not(target_os = "macos"))]
fn write_secret(_account: &str, _secret: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn delete_secret(account: &str) -> anyhow::Result<()> {
    let output = Command::new("/usr/bin/security")
        .args([
            "delete-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
        ])
        .output()
        .with_context(|| "无法调用 macOS 钥匙串 security 命令")?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let normalized = stderr.to_ascii_lowercase();
    if normalized.contains("could not be found") || normalized.contains("not found") {
        return Ok(());
    }
    anyhow::bail!("删除 macOS 钥匙串失败：{}", stderr.trim())
}

#[cfg(not(target_os = "macos"))]
fn delete_secret(_account: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_secret(account: &str) -> anyhow::Result<Option<String>> {
    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            account,
            "-w",
        ])
        .output()
        .with_context(|| "无法调用 macOS 钥匙串 security 命令")?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!value.is_empty()).then_some(value))
}

#[cfg(not(target_os = "macos"))]
fn read_secret(_account: &str) -> anyhow::Result<Option<String>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychain_refs_roundtrip_account_names() {
        let account = relay_profile_api_key_account("供应商 A/default");

        assert_eq!(account, "relay-profile:____A_default");
        assert!(is_keychain_ref(&keychain_ref(&account)));
        assert_eq!(
            keychain_account_from_ref(&keychain_ref(&account)).as_deref(),
            Some(account.as_str())
        );
        assert_eq!(
            keychain_account_from_ref(&keychain_ref(local_backend_session_token_account()))
                .as_deref(),
            Some(local_backend_session_token_account())
        );
        assert_eq!(
            keychain_account_from_ref(&keychain_ref(tencent_sms_secret_id_account())).as_deref(),
            Some(tencent_sms_secret_id_account())
        );
        assert_eq!(
            keychain_account_from_ref(&keychain_ref(tencent_sms_secret_key_account())).as_deref(),
            Some(tencent_sms_secret_key_account())
        );
    }

    #[test]
    fn plaintext_detector_ignores_keychain_refs() {
        let settings = BackendSettings {
            relay_api_key: keychain_ref(global_relay_api_key_account()),
            jiyi_identity_sync_api_key: keychain_ref(identity_sync_api_key_account()),
            relay_profiles: vec![RelayProfile {
                auth_contents: serde_json::to_string_pretty(&serde_json::json!({
                    "OPENAI_API_KEY": keychain_ref("relay-profile:default")
                }))
                .unwrap(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };

        assert!(!settings_contain_plaintext_api_key(&settings));
        assert!(settings_contain_keychain_ref(&settings));
    }

    #[test]
    fn plaintext_detector_flags_identity_sync_api_key() {
        let settings = BackendSettings {
            jiyi_identity_sync_api_key: "sync-secret".to_string(),
            ..BackendSettings::default()
        };

        assert!(settings_contain_plaintext_api_key(&settings));
        assert!(!settings_contain_keychain_ref(&settings));
    }

    #[test]
    fn materialize_profile_secrets_replaces_refs_with_fallback_without_keychain() {
        let mut profile = RelayProfile {
            api_key: keychain_ref("relay-profile:default"),
            auth_contents: serde_json::to_string_pretty(&serde_json::json!({
                "OPENAI_API_KEY": keychain_ref("relay-profile:default")
            }))
            .unwrap(),
            ..RelayProfile::default()
        };

        let changed = materialize_relay_profile_secrets(&mut profile, "sk-test").unwrap();

        assert!(changed);
        assert_eq!(profile.api_key, "sk-test");
        assert!(profile.auth_contents.contains("sk-test"));
    }
}
