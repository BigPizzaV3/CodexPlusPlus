//! 一家供应商里的多个凭据：多个登录账号 / 多个 Key 的展开、分法、单个凭据的设置，以及加账号、移除账号。
//!
//! Key 池的状态是进程级的：每个用例用自己的供应商 id，互不干扰。

use std::collections::BTreeMap;
use std::time::Instant;

use codex_plus_core::gateway::credentials;
use codex_plus_core::gateway::key_pool;
use codex_plus_core::gateway::quota::{self, ProviderQuota, QuotaWindow};
use codex_plus_core::gateway::store::{
    CredentialRotation, CredentialSettings, GatewayProvider, GatewaySettings, GatewayStore,
    KeyRotationMode, ProviderCredentials,
};
use codex_plus_core::settings::{RelayAccount, RelayApiKey, RelayMode, RelayProfile, RelayProtocol};
use serde_json::json;

/// 一份 Copilot 登录（token 远未过期，请求前不用换新）。
fn copilot_auth(login: &str) -> String {
    json!({
        "codex_plus_account": "copilot",
        "login": login,
        "github_token": format!("gh-{login}"),
        "copilot_token": format!("cp-{login}"),
        "expires_at_ms": u64::MAX / 2,
        "api_base": "https://api.example.test",
        "sku": "",
    })
    .to_string()
}

fn copilot_relay(id: &str, logins: &[&str]) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: format!("Copilot · {}", logins[0]),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::ChatCompletions,
        base_url: "https://api.example.test#".to_string(),
        api_key: format!("cp-{}", logins[0]),
        auth_contents: copilot_auth(logins[0]),
        model_list: "gpt-a\ngpt-b".to_string(),
        accounts: logins[1..]
            .iter()
            .enumerate()
            .map(|(index, login)| RelayAccount {
                id: format!("account-{}", index + 2),
                name: format!("Copilot · {login}"),
                auth_contents: copilot_auth(login),
            })
            .collect(),
        ..RelayProfile::default()
    }
}

fn keyed_relay(id: &str, keys: &[&str]) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_string(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::Responses,
        base_url: "https://api.example.test/v1".to_string(),
        api_key: keys[0].to_string(),
        api_keys: keys
            .iter()
            .enumerate()
            .map(|(index, key)| RelayApiKey {
                id: format!("key-{}", index + 1),
                name: format!("Key {}", index + 1),
                api_key: key.to_string(),
            })
            .collect(),
        active_api_key_id: "key-1".to_string(),
        ..RelayProfile::default()
    }
}

fn store_with(providers: Vec<GatewayProvider>) -> (tempfile::TempDir, GatewayStore) {
    let directory = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(directory.path().join("gateway.json"));
    let mut gateway = GatewaySettings::default();
    gateway.providers = providers;
    store.save(&gateway).unwrap();
    (directory, store)
}

fn order(
    relay: &RelayProfile,
    model: &str,
    credentials: &ProviderCredentials,
    conversation: Option<&str>,
) -> Vec<String> {
    key_pool::order_credentials(
        relay,
        model,
        credentials,
        KeyRotationMode::Sticky,
        conversation,
        Instant::now(),
        true,
    )
    .available
    .into_iter()
    .map(|candidate| candidate.key.key_id)
    .collect()
}

#[test]
fn single_account_stays_exactly_as_before() {
    let relay = copilot_relay("cred-single", &["alice"]);
    let keys = key_pool::expand_keys(&relay);
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key.key_id, key_pool::PRIMARY_KEY_ID);
    assert_eq!(keys[0].relay, relay, "只有一个账号时候选就是供应商本身");
    let saved = serde_json::to_value(&relay).unwrap();
    assert!(saved.get("accounts").is_none(), "没有其余账号时不写 accounts");
    let provider = GatewayProvider::new(relay, "login:copilot");
    let saved = serde_json::to_value(&provider).unwrap();
    assert!(saved.get("credentials").is_none(), "没设过时不写 credentials");
}

#[test]
fn every_account_becomes_its_own_candidate() {
    let relay = copilot_relay("cred-expand", &["alice", "bob", "carol"]);
    let keys = key_pool::expand_keys(&relay);
    let ids = keys.iter().map(|key| key.key.key_id.as_str()).collect::<Vec<_>>();
    assert_eq!(ids, ["primary", "account-2", "account-3"]);
    assert_eq!(keys[0].relay.auth_contents, copilot_auth("alice"));
    assert_eq!(keys[1].relay.auth_contents, copilot_auth("bob"));
    assert_eq!(keys[1].relay.account_id, "account-2");
    assert!(keys[1].relay.api_key.is_empty(), "其余账号不沿用第一个账号的 token");
    assert!(keys.iter().all(|key| key.relay.accounts.is_empty()), "候选不再带账号列表");
    assert_eq!(keys[2].relay.credential_ref(), "cred-expand~account-3");
    assert_ne!(keys[0].key.fingerprint, keys[1].key.fingerprint);
}

#[tokio::test]
async fn other_accounts_get_their_token_from_their_own_login() {
    let relay = copilot_relay("cred-prepare", &["alice", "bob"]);
    let bob = key_pool::expand_keys(&relay).remove(1).relay;
    let prepared = codex_plus_core::gateway::accounts::prepare_relay(&bob).await.unwrap();
    assert_eq!(prepared.api_key, "cp-bob");
    assert_eq!(prepared.base_url, "https://api.example.test#");
}

#[test]
fn disabled_and_model_limited_credentials_are_skipped() {
    let relay = copilot_relay("cred-filter", &["alice", "bob", "carol"]);
    let mut settings = BTreeMap::new();
    settings.insert(
        "primary".to_string(),
        CredentialSettings {
            disabled: true,
            ..Default::default()
        },
    );
    settings.insert(
        "account-2".to_string(),
        CredentialSettings {
            models: vec!["GPT-A".to_string()],
            ..Default::default()
        },
    );
    let credentials = ProviderCredentials {
        rotation: None,
        settings,
        order: Vec::new(),
    };
    assert_eq!(order(&relay, "gpt-a", &credentials, None), ["account-2", "account-3"]);
    assert_eq!(order(&relay, "gpt-b", &credentials, None), ["account-3"]);
}

#[test]
fn rotate_spreads_new_conversations_but_a_conversation_stays_put() {
    let relay = copilot_relay("cred-rotate", &["alice", "bob", "carol"]);
    let credentials = ProviderCredentials {
        rotation: Some(CredentialRotation::Rotate),
        settings: BTreeMap::new(),
        order: Vec::new(),
    };
    let firsts = (0..3)
        .map(|_| order(&relay, "gpt-a", &credentials, None)[0].clone())
        .collect::<Vec<_>>();
    assert_eq!(firsts, ["primary", "account-2", "account-3"]);

    let key = key_pool::expand_keys(&relay)
        .into_iter()
        .find(|key| key.key.key_id == "account-2")
        .unwrap()
        .key;
    key_pool::record_success(&key, "gpt-a", Some("conv-r"));
    for _ in 0..3 {
        assert_eq!(order(&relay, "gpt-a", &credentials, Some("conv-r"))[0], "account-2");
    }
}

#[test]
fn weighted_follows_the_weights() {
    let relay = copilot_relay("cred-weighted", &["alice", "bob"]);
    let mut settings = BTreeMap::new();
    settings.insert(
        "account-2".to_string(),
        CredentialSettings {
            weight: Some(3),
            ..Default::default()
        },
    );
    let credentials = ProviderCredentials {
        rotation: Some(CredentialRotation::Weighted),
        settings,
        order: Vec::new(),
    };
    let picks = (0..8)
        .map(|_| order(&relay, "gpt-a", &credentials, None)[0].clone())
        .collect::<Vec<_>>();
    assert_eq!(picks.iter().filter(|id| *id == "primary").count(), 2);
    assert_eq!(picks.iter().filter(|id| *id == "account-2").count(), 6);
}

#[test]
fn drain_returns_to_the_first_credential_once_it_recovers() {
    let relay = copilot_relay("cred-drain", &["alice", "bob"]);
    let credentials = ProviderCredentials {
        rotation: Some(CredentialRotation::Drain),
        settings: BTreeMap::new(),
        order: Vec::new(),
    };
    let keys = key_pool::expand_keys(&relay);
    // 会话落到第二个账号上（比如第一个刚被限流）：用完再换不粘，第一个可用时仍排第一。
    key_pool::record_success(&keys[1].key, "gpt-a", Some("conv-d"));
    assert_eq!(order(&relay, "gpt-a", &credentials, Some("conv-d"))[0], "primary");
    // 第一个额度用完：换第二个。
    key_pool::record_failure(
        &keys[0].key,
        "gpt-a",
        &key_pool::FailureClass::QuotaExhausted { reset_after: None },
        Instant::now(),
    );
    assert_eq!(order(&relay, "gpt-a", &credentials, Some("conv-d")), ["account-2"]);
}

#[test]
fn usage_prefers_the_account_with_most_left_and_caps_stop_an_account() {
    let relay = copilot_relay("cred-usage", &["alice", "bob"]);
    let views = relay.credential_views();
    let reading = |used: f64| ProviderQuota {
        windows: vec![QuotaWindow {
            window_minutes: None,
            used_percent: used,
            resets_at_ms: None,
            observed_at_ms: None,
        }],
        updated_at_ms: 1,
        windows_complete: true,
        ..Default::default()
    };
    quota::record_for_relay(&views[0], reading(80.0));
    quota::record_for_relay(&views[1], reading(20.0));
    assert_eq!(quota::used_percent_for_relay(&views[1]), Some(20.0), "额度按账号分开记");

    let usage = ProviderCredentials {
        rotation: Some(CredentialRotation::Usage),
        settings: BTreeMap::new(),
        order: Vec::new(),
    };
    assert_eq!(order(&relay, "gpt-a", &usage, None), ["account-2", "primary"]);

    let mut settings = BTreeMap::new();
    settings.insert(
        "primary".to_string(),
        CredentialSettings {
            used_percent_limit: Some(75),
            ..Default::default()
        },
    );
    let capped = ProviderCredentials {
        rotation: None,
        settings,
        order: Vec::new(),
    };
    assert_eq!(order(&relay, "gpt-a", &capped, None), ["account-2"], "用到上限的账号先不用");
    quota::forget("cred-usage");
    assert!(quota::used_percent_for_relay(&views[1]).is_none(), "删供应商时其余账号的额度一起清");
}

#[test]
fn keys_use_the_providers_own_rotation() {
    let relay = keyed_relay("cred-keys", &["sk-one-aaaaaaaa", "sk-two-bbbbbbbb"]);
    let credentials = ProviderCredentials {
        rotation: Some(CredentialRotation::Rotate),
        settings: BTreeMap::new(),
        order: Vec::new(),
    };
    let first = order(&relay, "m", &credentials, None)[0].clone();
    let second = order(&relay, "m", &credentials, None)[0].clone();
    assert_ne!(first, second, "全局是会话粘性，这家单独设成轮换");
}

#[test]
fn credential_refs_find_and_update_the_right_account() {
    let mut gateway = GatewaySettings::default();
    gateway.providers.push(GatewayProvider::new(
        copilot_relay("cred-ref", &["alice", "bob"]),
        "login:copilot",
    ));
    assert_eq!(
        gateway.credential("cred-ref~account-2").unwrap().auth_contents,
        copilot_auth("bob")
    );
    assert_eq!(gateway.credential("cred-ref").unwrap().auth_contents, copilot_auth("alice"));
    assert!(gateway.credential("cred-ref~nope").is_none());
    assert!(gateway.set_credential_auth("cred-ref~account-2", copilot_auth("bob2")));
    let relay = &gateway.provider("cred-ref").unwrap().relay;
    assert_eq!(relay.accounts[0].auth_contents, copilot_auth("bob2"));
    assert_eq!(relay.auth_contents, copilot_auth("alice"), "第一个账号不动");
}

#[test]
fn adding_an_account_joins_the_provider_and_relogin_only_updates() {
    let (_dir, store) = store_with(vec![
        GatewayProvider::new(copilot_relay("cred-add", &["alice"]), "login:copilot"),
        GatewayProvider::new(keyed_relay("cred-add-keys", &["sk-aaaaaaaaaaaa"]), "custom"),
    ]);
    let bob = copilot_relay("ignored", &["bob"]);
    let message = credentials::add_account(&store, "cred-add", &bob).unwrap();
    assert!(message.contains("2 个账号"), "{message}");
    let relay = store.load().unwrap().provider("cred-add").unwrap().relay.clone();
    assert_eq!(relay.accounts.len(), 1);
    assert_eq!(relay.accounts[0].id, "account-2");
    assert_eq!(relay.accounts[0].name, "Copilot · bob");

    // 同一个人再登录：只换 token。
    let mut again = copilot_relay("ignored", &["bob"]);
    again.auth_contents = again.auth_contents.replace("cp-bob", "cp-bob-new");
    let message = credentials::add_account(&store, "cred-add", &again).unwrap();
    assert!(message.starts_with("已更新"), "{message}");
    let relay = store.load().unwrap().provider("cred-add").unwrap().relay.clone();
    assert_eq!(relay.accounts.len(), 1);
    assert!(relay.accounts[0].auth_contents.contains("cp-bob-new"));

    // 第一个账号本人再登录也只换 token。
    let alice = copilot_relay("ignored", &["alice"]);
    assert!(credentials::add_account(&store, "cred-add", &alice).unwrap().starts_with("已更新"));

    // 种类不同的不能加进去。
    let error = credentials::add_account(&store, "cred-add-keys", &bob).unwrap_err();
    assert!(error.to_string().contains("不是同一种账号"), "{error}");
}

#[test]
fn removing_the_first_account_promotes_the_next_with_its_settings() {
    let mut provider = GatewayProvider::new(
        copilot_relay("cred-remove", &["alice", "bob", "carol"]),
        "login:copilot",
    );
    provider.credentials.settings.insert(
        "account-2".to_string(),
        CredentialSettings {
            weight: Some(5),
            ..Default::default()
        },
    );
    let (_dir, store) = store_with(vec![provider]);
    let gateway = credentials::remove_account(&store, "cred-remove", "primary").unwrap();
    let provider = gateway.provider("cred-remove").unwrap();
    assert_eq!(provider.relay.auth_contents, copilot_auth("bob"));
    assert_eq!(provider.relay.accounts.len(), 1);
    assert_eq!(provider.relay.accounts[0].id, "account-3");
    assert_eq!(
        provider.credentials.settings_for("primary").and_then(|settings| settings.weight),
        Some(5),
        "顶上来的账号带着自己的设置"
    );

    let gateway = credentials::remove_account(&store, "cred-remove", "account-3").unwrap();
    assert!(gateway.provider("cred-remove").unwrap().relay.accounts.is_empty());
    let error = credentials::remove_account(&store, "cred-remove", "primary").unwrap_err();
    assert!(error.to_string().contains("只有一个账号"), "{error}");
}

#[test]
fn credential_settings_are_cleaned_and_one_must_stay_enabled() {
    let (_dir, store) = store_with(vec![GatewayProvider::new(
        copilot_relay("cred-settings", &["alice", "bob"]),
        "login:copilot",
    )]);
    let gateway = credentials::set_credential_settings(
        &store,
        "cred-settings",
        "account-2",
        CredentialSettings {
            disabled: true,
            weight: Some(0),
            models: vec![" gpt-a ".to_string(), String::new()],
            used_percent_limit: Some(150),
        },
    )
    .unwrap();
    let saved = gateway
        .provider("cred-settings")
        .unwrap()
        .credentials
        .settings_for("account-2")
        .cloned()
        .unwrap();
    assert_eq!(saved.weight, Some(1));
    assert_eq!(saved.models, ["gpt-a"]);
    assert_eq!(saved.used_percent_limit, Some(100));

    let error = credentials::set_credential_settings(
        &store,
        "cred-settings",
        "primary",
        CredentialSettings {
            disabled: true,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("至少要留一个"), "{error}");

    // 恢复成默认就不留这一项。
    let gateway = credentials::set_credential_settings(
        &store,
        "cred-settings",
        "account-2",
        CredentialSettings::default(),
    )
    .unwrap();
    assert!(gateway.provider("cred-settings").unwrap().credentials.is_empty());

    let gateway =
        credentials::set_rotation(&store, "cred-settings", Some(CredentialRotation::Drain)).unwrap();
    let provider = gateway.provider("cred-settings").unwrap();
    assert_eq!(credentials::rotation(provider, &gateway), CredentialRotation::Drain);
    let views = credentials::views(provider);
    assert_eq!(views.len(), 2);
    assert_eq!(views[1].kind, "account");
    assert_eq!(views[1].id, "account-2");
}

#[test]
fn dragged_order_decides_who_goes_first() {
    let relay = copilot_relay("cred-order", &["alice", "bob", "carol"]);
    let credentials = ProviderCredentials {
        order: vec!["account-3".to_string(), "primary".to_string()],
        ..Default::default()
    };
    assert_eq!(
        order(&relay, "gpt-a", &credentials, None),
        ["account-3", "primary", "account-2"],
        "排过的在前，没排到的照原顺序跟在后面"
    );

    let mut provider = GatewayProvider::new(relay, "login:copilot");
    provider.credentials = credentials;
    let ids = credentials::views(&provider).into_iter().map(|view| view.id).collect::<Vec<_>>();
    assert_eq!(ids, ["account-3", "primary", "account-2"], "弹窗里的顺序和请求时一样");
}

#[test]
fn saving_an_order_must_name_every_credential_and_removal_keeps_it_tidy() {
    let (_dir, store) = store_with(vec![GatewayProvider::new(
        copilot_relay("cred-order-save", &["alice", "bob", "carol"]),
        "login:copilot",
    )]);
    let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    assert!(credentials::set_order(&store, "cred-order-save", &ids(&["account-2", "primary"])).is_err());
    assert!(credentials::set_order(&store, "cred-order-save", &ids(&["account-2", "primary", "primary"])).is_err());
    let gateway = credentials::set_order(&store, "cred-order-save", &ids(&["account-2", "account-3", "primary"])).unwrap();
    assert_eq!(gateway.provider("cred-order-save").unwrap().credentials.order, ids(&["account-2", "account-3", "primary"]));

    // 移除第一个账号：bob（account-2）顶上来成为第一个账号，排序里它的位置不变、改叫 primary。
    let gateway = credentials::remove_account(&store, "cred-order-save", "primary").unwrap();
    assert_eq!(gateway.provider("cred-order-save").unwrap().credentials.order, ids(&["primary", "account-3"]));
    let gateway = credentials::remove_account(&store, "cred-order-save", "account-3").unwrap();
    assert_eq!(gateway.provider("cred-order-save").unwrap().credentials.order, ids(&["primary"]));
}
