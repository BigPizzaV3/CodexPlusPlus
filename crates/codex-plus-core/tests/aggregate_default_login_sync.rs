//! 聚合默认成员的官方登录同步进 Codex（`syncDefaultMemberLogin`）。
//!
//! 背景：聚合的请求都发往本地代理，切换时只往 live auth.json 写一个代理 Key，所以
//! Codex 侧显示"未登录"，官方成员的账号态与额度都不体现。打开这个开关后，切换聚合
//! 会把默认成员的 ChatGPT 凭据一并写进 live auth.json（代理 Key 与其它字段保留）。
//!
//! 这里钉住四件事：
//!   1. 默认关：行为与历史完全一致（只有代理 Key）；
//!   2. 开着 + 默认成员是官方登录：`tokens` / `auth_mode` 写进去，代理 Key 不丢；
//!   3. 同一账号优先用 live 里更新的一代凭据（Codex 会轮换 refresh token）；
//!   4. 默认成员不是官方登录、或用户已在 Codex 登录了别的账号时不覆盖，并清掉残留。

use base64::Engine;
use codex_plus_core::paths::set_settings_path_for_tests;
use codex_plus_core::relay_config::apply_relay_profile_to_home_with_switch_rules;
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings, RelayMode,
    RelayProfile, RelaySessionProvider, SettingsStore,
};
use serde_json::{Value, json};

const MEMBER_ID: &str = "official-account";
const AGGREGATE_ID: &str = "agg";
const PROXY_KEY: &str = "codex-plus-aggregate";

/// 造一个不会触发刷新的 access token（`exp` 放到 2100 年）。
fn access_token(account_id: &str) -> String {
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        json!({
            "exp": 4_102_444_800_i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": account_id },
        })
        .to_string(),
    );
    format!("{header}.{payload}.sig")
}

/// 真实 auth.json 的形状：`tokens` 里有 `id_token`，顶层有 `last_refresh`。
/// Codex 解析登录态时 `id_token` 是必填，缺了会报 `missing field id_token`。
fn account_auth(access: &str, refresh: &str, account_id: &str) -> String {
    json!({
        "auth_mode": "chatgpt",
        "last_refresh": "2026-10-14T00:00:00Z",
        "tokens": {
            "access_token": access,
            "refresh_token": refresh,
            "account_id": account_id,
            "id_token": id_token(account_id),
        }
    })
    .to_string()
}

/// 造一个最小的 id_token（带 chatgpt_account_id claim）。
fn id_token(account_id: &str) -> String {
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        json!({
            "exp": 4_102_444_800_i64,
            "https://api.openai.com/auth": { "chatgpt_account_id": account_id },
            "email": "user@example.test",
        })
        .to_string(),
    );
    format!("{header}.{payload}.sig")
}

fn official_member(account_id: &str, refresh: &str) -> RelayProfile {
    RelayProfile {
        id: MEMBER_ID.to_string(),
        name: "官方账号".to_string(),
        relay_mode: RelayMode::Official,
        official_mix_api_key: false,
        model: "gpt-5.6-sol".to_string(),
        model_list: "gpt-5.6-sol".to_string(),
        auth_contents: account_auth(&access_token(account_id), refresh, account_id),
        ..RelayProfile::default()
    }
}

fn aggregate_profile(sync: bool, default_member: &str) -> AggregateRelayProfile {
    AggregateRelayProfile {
        id: AGGREGATE_ID.to_string(),
        name: "聚合供应商".to_string(),
        session_provider: RelaySessionProvider::Custom,
        model_aggregation_enabled: true,
        strategy: AggregateRelayStrategy::Failover,
        members: vec![AggregateRelayMember {
            relay_id: default_member.to_string(),
            weight: 1,
        }],
        routes: Vec::new(),
        default_relay_id: default_member.to_string(),
        sync_default_member_login: sync,
    }
}

/// 聚合档案本身（成员列表只存在 `aggregateRelayProfiles` sidecar 里）。
fn relay_profile() -> RelayProfile {
    RelayProfile {
        id: AGGREGATE_ID.to_string(),
        name: "聚合供应商".to_string(),
        relay_mode: RelayMode::Aggregate,
        api_key: PROXY_KEY.to_string(),
        model: "cpx-official-account/gpt-5.6-sol".to_string(),
        ..RelayProfile::default()
    }
}

/// 写一份 settings 并应用一次聚合切换；返回切换后的 home 目录（含 auth.json）。
/// `set_settings_path_for_tests` 是进程级全局状态：这些用例必须串行跑，否则互相
/// 覆盖 settings 路径（表现是"开关关着也读到了别的用例的同步配置"）。
static SETTINGS_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn apply_switch(
    member: RelayProfile,
    aggregate: AggregateRelayProfile,
    live_auth: Option<Value>,
) -> (tempfile::TempDir, Value) {
    let default_member = aggregate.default_relay_id.clone();
    let sync = aggregate.sync_default_member_login;
    let _serial = SETTINGS_GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    if let Some(auth) = live_auth {
        std::fs::write(
            home.join("auth.json"),
            serde_json::to_string_pretty(&auth).unwrap(),
        )
        .unwrap();
    }

    let settings_path = dir.path().join("settings.json");
    let _guard = set_settings_path_for_tests(Some(settings_path.clone()));
    let mut settings = BackendSettings::default();
    settings.relay_profiles = vec![member, relay_profile()];
    settings.active_relay_id = AGGREGATE_ID.to_string();
    settings.active_aggregate_relay_id = AGGREGATE_ID.to_string();
    settings.aggregate_relay_profiles = vec![aggregate];
    std::fs::write(&settings_path, serde_json::to_string(&settings).unwrap()).unwrap();
    apply_relay_profile_to_home_with_switch_rules(&home, &relay_profile(), "").unwrap();
    let auth: Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("auth.json")).unwrap()).unwrap();
    (dir, auth)
}

/// 默认关：只写代理 Key，不主动写入成员登录态（与改动前逐字一致）。
#[test]
fn sync_disabled_keeps_proxy_key_only() {
    let (_dir, auth) = apply_switch(official_member("acct-1", "refresh-1"), aggregate_profile(false, MEMBER_ID), None);

    assert_eq!(auth["OPENAI_API_KEY"], PROXY_KEY);
    assert!(auth.get("tokens").is_none(), "开关关着时不该写入 tokens：{auth}");
    assert!(auth.get("auth_mode").is_none(), "{auth}");
}

/// 默认关时也不能把用户已有的登录态抹掉（历史行为：live 优先合并）。
#[test]
fn sync_disabled_keeps_existing_live_login() {
    let (_dir, auth) = apply_switch(
        official_member("acct-1", "refresh-1"),
        aggregate_profile(false, MEMBER_ID),
        Some(json!({
            "auth_mode": "chatgpt",
            "tokens": { "access_token": "live", "refresh_token": "live", "account_id": "acct-user" }
        })),
    );

    assert_eq!(auth["OPENAI_API_KEY"], PROXY_KEY);
    assert_eq!(auth["tokens"]["account_id"], "acct-user", "{auth}");
}

/// 开着 + 默认成员是官方登录：账号登录态与代理 Key 同时存在。
#[test]
fn sync_enabled_writes_official_login_next_to_proxy_key() {
    let member = official_member("acct-1", "refresh-1");
    let (_dir, auth) = apply_switch(member.clone(), aggregate_profile(true, MEMBER_ID), None);

    assert_eq!(auth["OPENAI_API_KEY"], PROXY_KEY, "代理 Key 必须保留：{auth}");
    assert_eq!(auth["auth_mode"], "chatgpt");
    assert_eq!(auth["tokens"]["account_id"], "acct-1");
    assert_eq!(auth["tokens"]["refresh_token"], "refresh-1");
    assert_eq!(
        auth["tokens"]["access_token"],
        json!(access_token("acct-1")),
        "应写入默认成员的 access token"
    );
}

/// Codex 会刷新并轮换 refresh token：同账号要以 live 的那一代为准。
#[test]
fn sync_prefers_newer_live_credentials_for_same_account() {
    let (_dir, auth) = apply_switch(
        official_member("acct-1", "refresh-old"),
        aggregate_profile(true, MEMBER_ID),
        Some(json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token("acct-1"),
                "refresh_token": "refresh-new",
                "account_id": "acct-1",
            }
        })),
    );

    assert_eq!(
        auth["tokens"]["refresh_token"], "refresh-new",
        "同账号应采纳 live 里更新的 refresh token：{auth}"
    );
}

#[test]
fn does_not_replace_a_different_live_account() {
    let (_dir, auth) = apply_switch(
        official_member("acct-1", "refresh-1"),
        aggregate_profile(true, MEMBER_ID),
        Some(json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": access_token("acct-other"),
                "refresh_token": "refresh-other",
                "account_id": "acct-other",
            }
        })),
    );

    assert_eq!(
        auth["tokens"]["account_id"], "acct-other",
        "不该把用户在 Codex 里登录的账号顶掉：{auth}"
    );
    assert_eq!(auth["tokens"]["refresh_token"], "refresh-other");
    assert_eq!(
        auth["tokens"]["access_token"],
        json!(access_token("acct-other")),
        "别的账号的 access token 也不能被成员凭据覆盖：{auth}"
    );
}

/// 成员是官方登录、live 里只有代理 Key（没有 tokens）时：照样把账号写进去。
#[test]
fn sync_enabled_writes_login_when_live_has_only_proxy_key() {
    let (_dir, auth) = apply_switch(
        official_member("acct-1", "refresh-1"),
        aggregate_profile(true, MEMBER_ID),
        Some(json!({ "OPENAI_API_KEY": "codex-plus-aggregate" })),
    );

    assert_eq!(auth["OPENAI_API_KEY"], PROXY_KEY);
    assert_eq!(auth["auth_mode"], "chatgpt");
    assert_eq!(auth["tokens"]["account_id"], "acct-1");
    assert_eq!(auth["tokens"]["refresh_token"], "refresh-1");
}

/// 回归：同步后 live `auth.json` 必须保留成员快照里的 `id_token`（Codex 必填）与
/// `last_refresh`。少写 `id_token` 时 Codex 报 `missing field id_token`，界面依旧未登录。
#[test]
fn sync_keeps_member_id_token_and_last_refresh() {
    let member = official_member("acct-1", "refresh-1");
    let (_dir, auth) = apply_switch(member, aggregate_profile(true, MEMBER_ID), None);

    assert_eq!(
        auth["tokens"]["id_token"],
        json!(id_token("acct-1")),
        "id_token 必须带过去，否则 Codex 解析登录态失败：{auth}"
    );
    assert!(
        auth.get("last_refresh").and_then(Value::as_str).is_some(),
        "last_refresh 也要带上：{auth}"
    );
    assert_eq!(auth["auth_mode"], "chatgpt");
}

/// 回归：只写 tokens 不够——provider 的 `requires_openai_auth = false` 时 Codex 根本不读登录态，
/// 账号菜单只显示 provider 名（"custom"）。同步生效时必须切到「官方 + API」形态：
/// `requires_openai_auth = true`，请求仍用代理 Key（experimental_bearer_token）发往本地代理。
#[test]
fn sync_enabled_marks_provider_as_requiring_openai_auth() {
    let (dir, _auth) = apply_switch(
        official_member("acct-1", "refresh-1"),
        aggregate_profile(true, MEMBER_ID),
        None,
    );
    let config = std::fs::read_to_string(dir.path().join("codex-home/config.toml")).unwrap();

    assert!(config.contains("requires_openai_auth = true"), "{config}");
    assert!(
        config.contains(&format!("experimental_bearer_token = \"{PROXY_KEY}\"")),
        "请求仍须用代理 Key：{config}"
    );
}

/// 开关关着：保持纯 API 形态（requires_openai_auth = false），与历史行为一致。
#[test]
fn provider_stays_api_only_when_sync_disabled() {
    let (dir, _auth) = apply_switch(
        official_member("acct-1", "refresh-1"),
        aggregate_profile(false, MEMBER_ID),
        None,
    );
    let config = std::fs::read_to_string(dir.path().join("codex-home/config.toml")).unwrap();

    assert!(config.contains("requires_openai_auth = false"), "{config}");
}
