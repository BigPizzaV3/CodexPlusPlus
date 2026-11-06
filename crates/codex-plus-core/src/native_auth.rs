//! Explicit native ChatGPT identity selection and canonical gateway account binding.

use std::path::Path;

use anyhow::Context;
use serde_json::Value;

use crate::gateway::store::{GatewayProvider, GatewayStore};
use crate::settings::{BackendSettings, NativeAuthSelection, RelayProfile, SettingsStore};

fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .ok()
        .or_else(|| {
            base64::engine::general_purpose::URL_SAFE
                .decode(payload.as_bytes())
                .ok()
        })?;
    serde_json::from_slice(&decoded).ok()
}

fn token_claim(auth: &Value, key: &str) -> Option<String> {
    let tokens = auth.get("tokens")?;
    for name in ["id_token", "access_token"] {
        if let Some(token) = tokens.get(name).and_then(Value::as_str) {
            if let Some(claims) = jwt_claims(token) {
                let value = claims
                    .get(key)
                    .or_else(|| claims.pointer(key))
                    .and_then(Value::as_str);
                if let Some(value) = value {
                    if !value.trim().is_empty() {
                        return Some(value.trim().to_string());
                    }
                }
            }
        }
    }
    None
}

/// Stable, opaque account scope used for sharing official quota readings.
/// An incomplete identity deliberately returns None and is never merged with another account.
pub fn account_scope(auth_contents: &str) -> Option<String> {
    let auth: Value = serde_json::from_str(auth_contents).ok()?;
    let tokens = auth.get("tokens")?;
    let workspace = tokens
        .get("account_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| token_claim(&auth, "/https:~1~1api.openai.com~1auth/chatgpt_account_id"))?;
    let subject = token_claim(&auth, "sub")
        .or_else(|| token_claim(&auth, "/https:~1~1api.openai.com~1auth/chatgpt_user_id"))?;
    let issuer = token_claim(&auth, "iss").unwrap_or_else(|| "https://auth.openai.com".to_string());
    Some(format!("{issuer}|{subject}|{workspace}"))
}

fn token_pair(auth_contents: &str) -> Option<(String, String, String)> {
    let auth: Value = serde_json::from_str(auth_contents).ok()?;
    let tokens = auth.get("tokens")?;
    Some((
        tokens
            .get("account_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        tokens
            .get("access_token")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        tokens
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
    ))
}

/// Strict account equality. Complete subject/workspace identity wins; otherwise only
/// the exact same token generation is accepted as a conservative legacy fallback.
pub fn same_account(left: &str, right: &str) -> bool {
    match (account_scope(left), account_scope(right)) {
        (Some(left), Some(right)) => return left == right,
        (Some(_), None) | (None, Some(_)) => return false,
        (None, None) => {}
    }
    let Some((left_account, left_access, left_refresh)) = token_pair(left) else {
        return false;
    };
    let Some((right_account, right_access, right_refresh)) = token_pair(right) else {
        return false;
    };
    !left_account.is_empty()
        && left_account == right_account
        && !left_access.is_empty()
        && left_access == right_access
        && !left_refresh.is_empty()
        && left_refresh == right_refresh
        && serde_json::from_str::<Value>(left)
            .ok()
            .and_then(|value| value.pointer("/tokens/id_token").cloned())
            == serde_json::from_str::<Value>(right)
                .ok()
                .and_then(|value| value.pointer("/tokens/id_token").cloned())
}

pub fn same_credential_generation(current: &str, expected: &str) -> bool {
    same_account(current, expected)
        && token_pair(current)
            .zip(token_pair(expected))
            .is_some_and(|(current, expected)| current.1 == expected.1 && current.2 == expected.2)
}

const NATIVE_GENERATION: &str = "nativeAuthGeneration";

fn credential_generation(auth: &str) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    let pair = token_pair(auth).context("ChatGPT 登录缺少 tokens")?;
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&pair)?)))
}

pub fn is_default_home(home: &Path) -> bool {
    let default = crate::codex_home::default_codex_home_dir();
    home == default
        || std::fs::canonicalize(home)
            .ok()
            .zip(std::fs::canonicalize(default).ok())
            .is_some_and(|(home, default)| home == default)
}

fn read_auth(home: &Path) -> anyhow::Result<Option<String>> {
    let path = home.join("auth.json");
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            auth_object(&contents).with_context(|| format!("无法解析 {}", path.display()))?;
            Ok(Some(contents))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("无法读取 {}", path.display())),
    }
}

fn auth_object(contents: &str) -> anyhow::Result<serde_json::Map<String, Value>> {
    if contents.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    serde_json::from_str::<Value>(contents)?
        .as_object()
        .cloned()
        .context("auth.json 必须是 JSON 对象")
}

fn preserve_auth(
    store: &GatewayStore,
    settings: &BackendSettings,
    live: &str,
) -> anyhow::Result<Option<String>> {
    if crate::official_login::official_login_from_auth_contents(live).is_none() {
        return Ok(None);
    }
    let generation = credential_generation(live)?;
    let (_, account_ref) = store.update(|gateway| {
        let selected = selection_account_ref(settings);
        let existing = gateway
            .providers
            .iter()
            .position(|provider| {
                selected == Some(provider.relay.id.as_str())
                    && same_account(&provider.relay.auth_contents, live)
            })
            .or_else(|| {
                gateway.providers.iter().position(|provider| {
                    same_credential_generation(&provider.relay.auth_contents, live)
                })
            });
        if let Some(index) = existing {
            let provider = &mut gateway.providers[index];
            let current = credential_generation(&provider.relay.auth_contents)?;
            let baseline = provider
                .extra
                .get(NATIVE_GENERATION)
                .and_then(Value::as_str);
            // A completed gateway rotation must not be replaced by its old native projection.
            if current != generation && baseline == Some(generation.as_str()) {
                return Ok(Some(provider.relay.id.clone()));
            }
            if current != generation && baseline.is_some_and(|baseline| baseline != current) {
                anyhow::bail!("原生登录与保存凭据同时更新，已停止切换以保留两边的新凭据");
            }
            provider.relay.auth_contents =
                merge_selected_auth(&provider.relay.auth_contents, Some(live))?;
            provider.extra.insert(
                NATIVE_GENERATION.to_string(),
                Value::String(generation.clone()),
            );
            return Ok(Some(provider.relay.id.clone()));
        }
        // A preserved native session remains a backup, not a separately rotating gateway login.
        let mut stored_auth = auth_object(live)?;
        stored_auth.insert(
            crate::official_login::GATEWAY_OWNED_MARKER.to_string(),
            Value::Bool(true),
        );
        stored_auth.insert(
            crate::official_login::NATIVE_OWNED_MARKER.to_string(),
            Value::Bool(true),
        );
        stored_auth.remove(crate::official_login::LENT_TO_CODEX_MARKER);
        let relay = RelayProfile {
            id: format!("native-{}", uuid::Uuid::new_v4()),
            name: token_claim(&Value::Object(stored_auth.clone()), "email")
                .map(|email| format!("ChatGPT {email}"))
                .unwrap_or_else(|| "ChatGPT".to_string()),
            relay_mode: crate::settings::RelayMode::Official,
            base_url: String::new(),
            auth_contents: serde_json::to_string_pretty(&stored_auth)?,
            ..RelayProfile::default()
        };
        let account_ref = relay.id.clone();
        let mut provider: GatewayProvider = serde_json::from_value(serde_json::json!({
            "enabled": false,
            "source": "native:external",
            "relay": relay,
        }))?;
        provider.extra.insert(
            NATIVE_GENERATION.to_string(),
            Value::String(generation.clone()),
        );
        gateway.providers.push(provider);
        Ok(Some(account_ref))
    })?;
    Ok(account_ref)
}

/// Ingest the native login before a caller replaces or clears any auth fields.
/// Isolated homes must never import synthetic or unrelated credentials into the main pool.
pub fn preserve_live_auth(home: &Path) -> anyhow::Result<Option<String>> {
    if !is_default_home(home) {
        return Ok(None);
    }
    let settings = SettingsStore::default().load()?;
    if settings.native_auth_selection.is_none() {
        return Ok(None);
    }
    let live = read_auth(home)?;
    preserve_auth(
        &GatewayStore::default(),
        &settings,
        live.as_deref().unwrap_or_default(),
    )
}

fn project_with_selection(
    live: &str,
    proposed: &str,
    selection: Option<&NativeAuthSelection>,
    selected_auth: Option<&str>,
) -> anyhow::Result<String> {
    let mut proposed = auth_object(proposed)?;
    let live = auth_object(live)?;
    // Non-login fields remain live-owned unless the caller intentionally changes them.
    for (key, value) in &live {
        if !["tokens", "auth_mode", "last_refresh"].contains(&key.as_str()) {
            proposed.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    let proposed = serde_json::to_string(&proposed)?;
    match selection {
        Some(NativeAuthSelection::None) => merge_selected_auth(&proposed, None),
        Some(NativeAuthSelection::Account { .. }) => merge_selected_auth(
            &proposed,
            Some(selected_auth.context("所选 ChatGPT 账号不存在")?),
        ),
        None if crate::official_login::official_login_from_auth_contents(
            &serde_json::to_string(&live)?,
        )
        .is_some() =>
        {
            merge_selected_auth(&proposed, Some(&serde_json::to_string(&live)?))
        }
        None => Ok(proposed),
    }
}

/// Final native auth projection. Callers must still compare live bytes before publishing.
pub fn project_native_auth(home: &Path, proposed: &str) -> anyhow::Result<String> {
    if !is_default_home(home) {
        return Ok(proposed.to_string());
    }
    let settings = SettingsStore::default().load()?;
    if settings.native_auth_selection.is_none() {
        return Ok(proposed.to_string());
    }
    let live = read_auth(home)?;
    let store = GatewayStore::default();
    preserve_auth(&store, &settings, live.as_deref().unwrap_or_default())?;
    let selected = selected_account_ref(&settings)
        .map(|account_ref| {
            let auth = gateway_auth(&account_ref)?;
            remember_projection(&store, &account_ref, &auth)?;
            Ok::<_, anyhow::Error>(auth)
        })
        .transpose()?;
    project_with_selection(
        live.as_deref().unwrap_or_default(),
        proposed,
        settings.native_auth_selection.as_ref(),
        selected.as_deref(),
    )
}

fn remember_projection(store: &GatewayStore, account_ref: &str, auth: &str) -> anyhow::Result<()> {
    let generation = credential_generation(auth)?;
    store.update(|gateway| {
        let provider = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == account_ref)
            .context("所选 ChatGPT 账号已被删除")?;
        if !same_credential_generation(&provider.relay.auth_contents, auth) {
            anyhow::bail!("所选 ChatGPT 凭据已更新，请重试");
        }
        provider
            .extra
            .insert(NATIVE_GENERATION.to_string(), Value::String(generation));
        Ok(())
    })?;
    Ok(())
}

pub(crate) fn gateway_auth(account_ref: &str) -> anyhow::Result<String> {
    gateway_auth_optional(account_ref)?
        .with_context(|| format!("找不到网关账号凭证：{account_ref}"))
}

pub(crate) fn gateway_auth_optional(account_ref: &str) -> anyhow::Result<Option<String>> {
    let gateway = GatewayStore::default().load()?;
    Ok(gateway
        .provider(account_ref)
        .map(|provider| {
            let mut auth = provider.relay.auth_contents.clone();
            if provider.source == "native:external"
                && let Ok(mut value) = serde_json::from_str::<Value>(&auth)
            {
                value[crate::official_login::NATIVE_OWNED_MARKER] = Value::Bool(true);
                auth = value.to_string();
            }
            auth
        })
        .filter(|contents| !contents.trim().is_empty()))
}

/// Resolve a profile's canonical account reference without changing persisted state.
pub fn resolve_profile_credentials(profile: &RelayProfile) -> anyhow::Result<RelayProfile> {
    if profile.official_account_ref.trim().is_empty() {
        return Ok(profile.clone());
    }
    let mut resolved = profile.clone();
    resolved.auth_contents = gateway_auth(profile.official_account_ref.trim())?;
    Ok(resolved)
}

/// Persist a rotated gateway credential only when the expected generation is still current.
pub fn persist_gateway_rotation(
    account_ref: &str,
    expected_auth: &str,
    updated_auth: &str,
) -> anyhow::Result<bool> {
    persist_gateway_rotation_with(
        &GatewayStore::default(),
        account_ref,
        expected_auth,
        updated_auth,
    )
}

fn persist_gateway_rotation_with(
    store: &GatewayStore,
    account_ref: &str,
    expected_auth: &str,
    updated_auth: &str,
) -> anyhow::Result<bool> {
    let account_ref = account_ref.trim();
    if account_ref.is_empty() || !same_account(expected_auth, updated_auth) {
        return Ok(false);
    }
    let (_, written) = store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == account_ref)
        else {
            return Ok(false);
        };
        if !same_credential_generation(&provider.relay.auth_contents, expected_auth) {
            return Ok(false);
        }
        provider.relay.auth_contents = updated_auth.to_string();
        Ok(true)
    })?;
    Ok(written)
}

/// Return whether a refreshed login may be projected into the native auth store.
pub fn native_write_allowed(home: &Path, auth_contents: &str) -> bool {
    if !is_default_home(home) {
        return false;
    }
    let Ok(settings) = SettingsStore::default().load() else {
        return false;
    };
    let selection = settings.native_auth_selection;
    match selection {
        Some(NativeAuthSelection::None) => false,
        Some(NativeAuthSelection::Account { account_ref }) => {
            gateway_auth(&account_ref).is_ok_and(|stored| same_account(&stored, auth_contents))
        }
        None => {
            // Legacy installations have no explicit choice. The caller still performs
            // an expected-generation check against the live auth file.
            let _ = home;
            true
        }
    }
}

pub fn selected_account_ref(settings: &BackendSettings) -> Option<String> {
    match settings.native_auth_selection.as_ref()? {
        NativeAuthSelection::None => None,
        NativeAuthSelection::Account { account_ref } => {
            (!account_ref.trim().is_empty()).then(|| account_ref.trim().to_string())
        }
    }
}

pub fn selection_allows_auto_lend(settings: &BackendSettings) -> bool {
    settings.native_auth_selection.is_none()
}

pub fn selection_account_ref(settings: &BackendSettings) -> Option<&str> {
    match settings.native_auth_selection.as_ref()? {
        NativeAuthSelection::None => None,
        NativeAuthSelection::Account { account_ref } => Some(account_ref.as_str()),
    }
}

pub(crate) fn merge_selected_auth(live: &str, selected: Option<&str>) -> anyhow::Result<String> {
    let mut object = auth_object(live)?;
    for key in ["tokens", "auth_mode", "last_refresh"] {
        object.remove(key);
    }
    match selected {
        Some(selected) => {
            let selected = auth_object(selected).context("选中的 ChatGPT 凭证格式无效")?;
            for key in ["tokens", "auth_mode", "last_refresh"] {
                if let Some(value) = selected.get(key) {
                    object.insert(key.to_string(), value.clone());
                }
            }
        }
        None => {
            for key in ["tokens", "auth_mode", "last_refresh"] {
                object.remove(key);
            }
        }
    }
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&Value::Object(object))?
    ))
}

/// Save the choice before publishing auth, with the settings store's conditional rollback.
pub fn set_native_auth_selection(account_ref: Option<&str>) -> anyhow::Result<()> {
    let home = crate::codex_home::default_codex_home_dir();
    let _guard = crate::official_login::native_auth_selection_write_guard()?;
    let settings_store = SettingsStore::default();
    let mut settings = settings_store.load()?;
    let auth_path = home.join("auth.json");
    let previous_auth = read_auth(&home)?;
    let gateway = GatewayStore::default();
    preserve_auth(
        &gateway,
        &settings,
        previous_auth.as_deref().unwrap_or_default(),
    )?;
    let selected_auth = account_ref
        .map(|account_ref| gateway_auth(account_ref.trim()))
        .transpose()?;
    if selected_auth.as_deref().is_some_and(|auth| {
        crate::official_login::official_login_from_auth_contents(auth).is_none()
    }) {
        anyhow::bail!("所选供应商没有可用的 ChatGPT 登录凭据");
    }
    let next_auth = merge_selected_auth(
        previous_auth.as_deref().unwrap_or_default(),
        selected_auth.as_deref(),
    )?;
    if let Some((account_ref, auth)) = account_ref.zip(selected_auth.as_deref()) {
        remember_projection(&gateway, account_ref.trim(), auth)?;
    }
    settings.native_auth_selection = Some(match account_ref {
        Some(account_ref) => NativeAuthSelection::Account {
            account_ref: account_ref.trim().to_string(),
        },
        None => NativeAuthSelection::None,
    });
    settings_store.save_with_commit(&settings, |_| {
        if read_auth(&home)? != previous_auth {
            anyhow::bail!("原生登录已被其他进程更新，已停止切换，请重试");
        }
        crate::settings::atomic_write(&auth_path, next_auth.as_bytes())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(claims: Value) -> String {
        use base64::Engine;
        format!(
            "e30.{}.",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&claims).unwrap())
        )
    }

    fn auth(subject: &str, generation: u8) -> String {
        json!({
            "auth_mode": "chatgpt",
            "last_refresh": "2026-11-05T00:00:00Z",
            "tokens": {
                "account_id": "shared-workspace",
                "id_token": jwt(json!({"sub": subject, "iss": "https://auth.openai.com"})),
                "access_token": format!("{subject}-access-{generation}"),
                "refresh_token": format!("{subject}-refresh-{generation}"),
            }
        })
        .to_string()
    }

    #[test]
    fn account_scope_reads_nested_user_and_workspace_from_access_jwt() {
        let contents = json!({"tokens": {
            "access_token": jwt(json!({
                "iss": "https://auth.openai.com",
                "https://api.openai.com/auth": {
                    "chatgpt_user_id": "person",
                    "chatgpt_account_id": "workspace",
                }
            })),
            "refresh_token": "refresh"
        }})
        .to_string();
        assert_eq!(
            account_scope(&contents).as_deref(),
            Some("https://auth.openai.com|person|workspace")
        );
        assert!(!same_account(&auth("alice", 0), &auth("bob", 0)));
        assert!(same_account(&auth("alice", 0), &auth("alice", 1)));
        assert!(!same_credential_generation(
            &auth("alice", 0),
            &auth("alice", 1)
        ));
        let incomplete = |email: &str| {
            json!({"tokens": {
                "account_id": "workspace",
                "access_token": "same-access",
                "refresh_token": "same-refresh",
                "id_token": jwt(json!({"email": email})),
            }})
            .to_string()
        };
        assert!(!same_account(
            &incomplete("alice@example.test"),
            &incomplete("bob@example.test")
        ));
        assert!(same_account(
            &incomplete("alice@example.test"),
            &incomplete("alice@example.test")
        ));
    }

    #[test]
    fn rotation_rejects_stale_generation_of_same_account() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let settings = BackendSettings::default();
        let original = auth("alice", 0);
        let account_ref = preserve_auth(&store, &settings, &original)
            .unwrap()
            .unwrap();
        assert!(
            persist_gateway_rotation_with(&store, &account_ref, &original, &auth("alice", 1))
                .unwrap()
        );
        assert!(
            !persist_gateway_rotation_with(&store, &account_ref, &original, &auth("alice", 2))
                .unwrap()
        );
        let gateway = store.load().unwrap();
        assert!(same_credential_generation(
            &gateway.provider(&account_ref).unwrap().relay.auth_contents,
            &auth("alice", 1)
        ));
    }

    #[test]
    fn explicit_switch_preserves_native_sessions_as_non_refreshable_backups() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let mut settings = BackendSettings::default();
        let first = preserve_auth(&store, &settings, &auth("alice", 0))
            .unwrap()
            .unwrap();
        let second = preserve_auth(&store, &settings, &auth("bob", 0))
            .unwrap()
            .unwrap();
        assert_ne!(first, second);
        settings.native_auth_selection = Some(NativeAuthSelection::Account {
            account_ref: first.clone(),
        });
        assert_eq!(
            preserve_auth(&store, &settings, &auth("alice", 1))
                .unwrap()
                .as_deref(),
            Some(first.as_str())
        );
        let gateway = store.load().unwrap();
        assert_eq!(gateway.providers.len(), 2);
        for provider in &gateway.providers {
            assert!(!provider.enabled);
            assert!(provider.subscription_policy.is_none());
            assert!(crate::official_login::is_native_owned_login(&provider.relay));
            assert!(!crate::official_login::is_gateway_owned_login(&provider.relay));
        }
        assert!(same_credential_generation(
            &gateway.provider(&second).unwrap().relay.auth_contents,
            &auth("bob", 0)
        ));
    }

    #[test]
    fn old_native_projection_does_not_revert_gateway_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let mut settings = BackendSettings::default();
        let original = auth("alice", 0);
        let account_ref = preserve_auth(&store, &settings, &original)
            .unwrap()
            .unwrap();
        settings.native_auth_selection = Some(NativeAuthSelection::Account {
            account_ref: account_ref.clone(),
        });
        assert!(
            persist_gateway_rotation_with(&store, &account_ref, &original, &auth("alice", 1))
                .unwrap()
        );
        preserve_auth(&store, &settings, &original).unwrap();
        let gateway = store.load().unwrap();
        assert!(same_credential_generation(
            &gateway.provider(&account_ref).unwrap().relay.auth_contents,
            &auth("alice", 1)
        ));
    }

    #[test]
    fn unselected_same_account_sessions_are_saved_separately() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let settings = BackendSettings::default();
        let first = preserve_auth(&store, &settings, &auth("alice", 0))
            .unwrap()
            .unwrap();
        let second = preserve_auth(&store, &settings, &auth("alice", 1))
            .unwrap()
            .unwrap();
        assert_ne!(first, second);
        let gateway = store.load().unwrap();
        assert!(same_credential_generation(
            &gateway.provider(&first).unwrap().relay.auth_contents,
            &auth("alice", 0)
        ));
    }

    #[test]
    fn explicit_none_and_account_override_stale_profile_login_only() {
        let mut live: Value = serde_json::from_str(&auth("alice", 0)).unwrap();
        live["custom"] = json!({"keep": true});
        live["OPENAI_API_KEY"] = json!("old-api");
        let mut proposed: Value = serde_json::from_str(&auth("bob", 0)).unwrap();
        proposed["OPENAI_API_KEY"] = json!("new-api");
        let none = project_with_selection(
            &live.to_string(),
            &proposed.to_string(),
            Some(&NativeAuthSelection::None),
            None,
        )
        .unwrap();
        let none: Value = serde_json::from_str(&none).unwrap();
        assert!(none.get("tokens").is_none());
        assert!(none.get("auth_mode").is_none());
        assert_eq!(none["custom"]["keep"], true);
        assert_eq!(none["OPENAI_API_KEY"], "new-api");
        let selection = NativeAuthSelection::Account {
            account_ref: "alice".to_string(),
        };
        let selected = project_with_selection(
            &live.to_string(),
            &proposed.to_string(),
            Some(&selection),
            Some(&auth("alice", 1)),
        )
        .unwrap();
        assert!(same_credential_generation(&selected, &auth("alice", 1)));
        let selected: Value = serde_json::from_str(&selected).unwrap();
        assert_eq!(selected["custom"]["keep"], true);
        assert_eq!(selected["OPENAI_API_KEY"], "new-api");
    }

    #[test]
    fn malformed_live_auth_is_rejected_and_isolated_home_is_untouched() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("auth.json"), b"{broken").unwrap();
        assert!(read_auth(temp.path()).is_err());
        assert!(merge_selected_auth("{broken", Some(&auth("alice", 0))).is_err());
        assert_eq!(
            project_native_auth(temp.path(), "unchanged").unwrap(),
            "unchanged"
        );
    }
}
