//! 一家供应商里的多个凭据：多个 API Key，或登录进同一家的多个账号。
//!
//! 第一个账号仍放在 `relay.auth_contents`，其余账号放在 `relay.accounts`，只有一个账号时和以前完全一样。
//! 请求时每个凭据各是一个候选（见 [`crate::gateway::key_pool::expand_keys`]），按这家设的分法排先后，
//! 每个凭据可以单独停用、设权重、限定模型，账号还能设额度上限
//! （见 [`crate::gateway::store::ProviderCredentials`]）。
//!
//! 在列表外面登录是新建一家；在一家里面「添加账号」才进这里的 [`add_account`]。

use anyhow::bail;
use serde::Serialize;

use crate::gateway::key_pool;
use crate::gateway::quota::ProviderQuota;
use crate::gateway::store::{
    CredentialRotation, CredentialSettings, GatewayProvider, GatewaySettings, GatewayStore,
};
use crate::settings::{RelayAccount, RelayProfile};

/// 登录型账号的种类：`chatgpt`（官方通道），或凭据里记的种类（`copilot` / `grok` / `chatgpt-api` /
/// 订阅的 `sub:<id>`）。API Key 供应商返回 None。
pub fn login_kind(relay: &RelayProfile) -> Option<String> {
    if crate::official_login::member_uses_official_login_transport(relay) {
        return Some("chatgpt".to_string());
    }
    crate::gateway::accounts::relay_account_kind(relay)
}

/// 凭据里认人的字段：同一个人再登录一次时只换 token，不重复加。
fn identity_field(kind: &str) -> &'static str {
    match kind {
        "copilot" => "login",
        "grok" => "email",
        "chatgpt-api" => "subject",
        _ => "account",
    }
}

/// 两份登录凭据是不是同一个人的同一个账号。
pub fn same_login(kind: &str, left: &str, right: &str) -> bool {
    if kind == "chatgpt" {
        return crate::gateway::accounts::same_chatgpt_login(left, right);
    }
    let identity = |contents: &str| {
        serde_json::from_str::<serde_json::Value>(contents)
            .ok()
            .and_then(|value| {
                value
                    .get(identity_field(kind))
                    .and_then(|field| field.as_str())
                    .map(str::to_string)
            })
            .filter(|value| !value.is_empty())
    };
    identity(left).is_some_and(|left| Some(left) == identity(right))
}

/// 把一次登录得到的账号加进 `provider_id` 这一家。
///
/// 账号种类要和这家一样（ChatGPT 账号只能加进 ChatGPT 那家）。同一个人已经在这家里（第一个账号或其余账号）时
/// 只换它的 token。返回给界面看的一句话。
pub fn add_account(
    store: &GatewayStore,
    provider_id: &str,
    login: &RelayProfile,
) -> anyhow::Result<String> {
    let Some(kind) = login_kind(login) else {
        bail!("这次登录没有拿到账号凭据");
    };
    let (_, (message, account_id)) = store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == provider_id)
        else {
            bail!("供应商不存在：{provider_id}");
        };
        let relay = &mut provider.relay;
        if login_kind(relay).as_deref() != Some(kind.as_str()) {
            bail!("「{}」不是同一种账号，不能加进去；要单独用请在列表里添加", relay.name);
        }
        if same_login(&kind, &relay.auth_contents, &login.auth_contents) {
            relay.auth_contents = login.auth_contents.clone();
            return Ok((format!("已更新「{}」的登录。", relay.name), String::new()));
        }
        if let Some(account) = relay
            .accounts
            .iter_mut()
            .find(|account| same_login(&kind, &account.auth_contents, &login.auth_contents))
        {
            account.auth_contents = login.auth_contents.clone();
            if !login.name.trim().is_empty() {
                account.name = login.name.clone();
            }
            return Ok((format!("已更新「{}」的登录。", account.name), account.id.clone()));
        }
        let number = (relay.accounts.len() + 2..)
            .find(|number| {
                let candidate = format!("account-{number}");
                !relay.accounts.iter().any(|account| account.id == candidate)
            })
            .unwrap_or_default();
        let id = format!("account-{number}");
        relay.accounts.push(RelayAccount {
            id: id.clone(),
            name: login.name.clone(),
            auth_contents: login.auth_contents.clone(),
        });
        // 新账号可能用到早先删掉的同名账号的额度设置：清掉。
        provider.credentials.settings.remove(&id);
        let total = provider.relay.accounts.len() + 1;
        Ok((
            format!("已把「{}」加进「{}」，这家现在有 {total} 个账号。", login.name, provider.relay.name),
            id,
        ))
    })?;
    if !account_id.is_empty() {
        key_pool::forget_key(provider_id, &account_id);
        crate::gateway::quota::forget_credential(&format!(
            "{provider_id}{}{account_id}",
            crate::settings::ACCOUNT_REF_SEPARATOR
        ));
    }
    Ok(message)
}

/// 从一家里移除一个账号。`credential_id` 是凭据 id（第一个账号是 `primary` / `official`）。
/// 只剩一个账号时不能移除（要不用了就删掉这家）。移除第一个账号时下一个账号顶上来。
pub fn remove_account(
    store: &GatewayStore,
    provider_id: &str,
    credential_id: &str,
) -> anyhow::Result<GatewaySettings> {
    let (gateway, forgotten) = store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == provider_id)
        else {
            bail!("供应商不存在：{provider_id}");
        };
        if provider.relay.accounts.is_empty() {
            bail!("这家只有一个账号；不用了请直接删除这家供应商");
        }
        let primary = is_primary_credential(credential_id);
        if primary {
            if crate::official_login::is_lent_to_codex(&provider.relay) {
                bail!("这个账号正借给 Codex 登录，先在 Codex 里换掉它再移除");
            }
            let next = provider.relay.accounts.remove(0);
            provider.relay.auth_contents = next.auth_contents;
            // 顶上来的账号换成第一个账号的凭据 id，它的设置跟着走；被移除的第一个账号的设置不留。
            let primary_id = primary_credential_id(&provider.relay);
            provider.credentials.settings.remove(key_pool::PRIMARY_KEY_ID);
            provider.credentials.settings.remove(key_pool::OFFICIAL_KEY_ID);
            if let Some(settings) = provider.credentials.settings.remove(&next.id) {
                provider.credentials.settings.insert(primary_id.clone(), settings);
            }
            // 排过的先后里：被移除的第一个账号去掉，顶上来的那个换成第一个账号的 id、位置不变。
            let order = &mut provider.credentials.order;
            order.retain(|id| id != key_pool::PRIMARY_KEY_ID && id != key_pool::OFFICIAL_KEY_ID);
            if let Some(slot) = order.iter_mut().find(|id| **id == next.id) {
                *slot = primary_id;
            }
            Ok(vec![
                key_pool::PRIMARY_KEY_ID.to_string(),
                key_pool::OFFICIAL_KEY_ID.to_string(),
                next.id,
            ])
        } else {
            let Some(position) = provider
                .relay
                .accounts
                .iter()
                .position(|account| account.id == credential_id)
            else {
                bail!("这个账号已经不在了，刷新后再试");
            };
            provider.relay.accounts.remove(position);
            provider.credentials.settings.remove(credential_id);
            provider.credentials.order.retain(|id| id != credential_id);
            Ok(vec![credential_id.to_string()])
        }
    })?;
    for key_id in forgotten {
        key_pool::forget_key(provider_id, &key_id);
        let reference = if is_primary_credential(&key_id) {
            provider_id.to_string()
        } else {
            format!("{provider_id}{}{key_id}", crate::settings::ACCOUNT_REF_SEPARATOR)
        };
        crate::gateway::quota::forget_credential(&reference);
    }
    Ok(gateway)
}

fn is_primary_credential(credential_id: &str) -> bool {
    credential_id == key_pool::PRIMARY_KEY_ID || credential_id == key_pool::OFFICIAL_KEY_ID
}

fn primary_credential_id(relay: &RelayProfile) -> String {
    if crate::official_login::member_uses_official_login_transport(relay) {
        key_pool::OFFICIAL_KEY_ID.to_string()
    } else {
        key_pool::PRIMARY_KEY_ID.to_string()
    }
}

/// 设这家的分法；None 表示跟全局的 Key 策略。
pub fn set_rotation(
    store: &GatewayStore,
    provider_id: &str,
    rotation: Option<CredentialRotation>,
) -> anyhow::Result<GatewaySettings> {
    let (gateway, ()) = store.update(|gateway| {
        provider_mut(gateway, provider_id)?.credentials.rotation = rotation;
        Ok(())
    })?;
    Ok(gateway)
}

/// 设这家各个 Key / 账号的先后（拖动排序）。`ids` 要恰好是这家现有的全部凭据 id。
pub fn set_order(
    store: &GatewayStore,
    provider_id: &str,
    ids: &[String],
) -> anyhow::Result<GatewaySettings> {
    let (gateway, ()) = store.update(|gateway| {
        let provider = provider_mut(gateway, provider_id)?;
        let mut current = key_pool::expand_keys(&provider.relay)
            .into_iter()
            .map(|candidate| candidate.key.key_id)
            .collect::<Vec<_>>();
        let mut wanted = ids.to_vec();
        current.sort();
        wanted.sort();
        wanted.dedup();
        if current != wanted || wanted.len() != ids.len() {
            bail!("Key / 账号有变化，刷新后再排");
        }
        provider.credentials.order = ids.to_vec();
        Ok(())
    })?;
    Ok(gateway)
}

/// 设一个凭据的单独设置；全是默认值时删掉这一项。权重限 1–1000，额度上限限 1–100%。
pub fn set_credential_settings(
    store: &GatewayStore,
    provider_id: &str,
    credential_id: &str,
    settings: CredentialSettings,
) -> anyhow::Result<GatewaySettings> {
    let mut settings = settings;
    settings.weight = settings.weight.map(|weight| weight.clamp(1, 1000));
    settings.used_percent_limit = settings.used_percent_limit.map(|limit| limit.clamp(1, 100));
    settings.models = settings
        .models
        .iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect();
    settings.models.dedup();
    let (gateway, ()) = store.update(|gateway| {
        let provider = provider_mut(gateway, provider_id)?;
        if !key_pool::expand_keys(&provider.relay)
            .iter()
            .any(|candidate| candidate.key.key_id == credential_id)
        {
            bail!("这个 Key / 账号已经不在了，刷新后再试");
        }
        if settings.disabled
            && key_pool::expand_keys(&provider.relay).iter().all(|candidate| {
                candidate.key.key_id == credential_id
                    || provider
                        .credentials
                        .settings_for(&candidate.key.key_id)
                        .is_some_and(|other| other.disabled)
            })
        {
            bail!("至少要留一个 Key / 账号不停用；整家不用请关掉这家供应商");
        }
        if settings.is_default() {
            provider.credentials.settings.remove(credential_id);
        } else {
            provider
                .credentials
                .settings
                .insert(credential_id.to_string(), settings);
        }
        Ok(())
    })?;
    Ok(gateway)
}

fn provider_mut<'a>(
    gateway: &'a mut GatewaySettings,
    provider_id: &str,
) -> anyhow::Result<&'a mut GatewayProvider> {
    gateway
        .providers
        .iter_mut()
        .find(|provider| provider.relay.id == provider_id)
        .ok_or_else(|| anyhow::anyhow!("供应商不存在：{provider_id}"))
}

/// 界面上的一个凭据（Key 原文、token 不出后端）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialView {
    /// 凭据 id：设置、移除按它找。
    pub id: String,
    /// `account`（登录账号）或 `key`（API Key）。
    pub kind: &'static str,
    /// 打码后的账号名或 Key。
    pub label: String,
    /// 订阅账号的套餐；Key 为空。
    pub plan: String,
    pub quota: Option<ProviderQuota>,
    pub settings: CredentialSettings,
}

/// 这家的全部凭据，按请求时的书写顺序（第一个账号 / 当前 Key 在前）。
pub fn views(provider: &GatewayProvider) -> Vec<CredentialView> {
    let relay = &provider.relay;
    let masked_keys = crate::gateway::apply::provider_keys(relay);
    let mut candidates = key_pool::expand_keys(relay);
    provider
        .credentials
        .sort_by_order(&mut candidates, |candidate| candidate.key.key_id.as_str());
    candidates
        .into_iter()
        .map(|candidate| {
            let view = &candidate.relay;
            let account = login_kind(view).is_some();
            let label = if account {
                let name = if view.account_id.is_empty() {
                    relay.name.clone()
                } else {
                    relay
                        .accounts
                        .iter()
                        .find(|entry| entry.id == view.account_id)
                        .map(|entry| entry.name.clone())
                        .unwrap_or_default()
                };
                crate::gateway::accounts::mask_account_name(&name)
            } else {
                masked_keys
                    .iter()
                    .find(|entry| entry.id == candidate.key.key_id)
                    .or(masked_keys.first().filter(|_| masked_keys.len() == 1))
                    .map(|entry| entry.masked.clone())
                    .unwrap_or_else(|| crate::gateway::apply::mask_api_key(&view.api_key))
            };
            CredentialView {
                id: candidate.key.key_id.clone(),
                kind: if account { "account" } else { "key" },
                label,
                plan: if account {
                    crate::gateway::accounts::account_plan(view)
                } else {
                    String::new()
                },
                quota: account
                    .then(|| crate::gateway::quota::quota_for_relay(view))
                    .flatten(),
                settings: provider
                    .credentials
                    .settings_for(&candidate.key.key_id)
                    .cloned()
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// 这家实际用的分法（界面显示用）。
pub fn rotation(provider: &GatewayProvider, gateway: &GatewaySettings) -> CredentialRotation {
    provider.credentials.rotation_or(gateway.key_policy.mode)
}
