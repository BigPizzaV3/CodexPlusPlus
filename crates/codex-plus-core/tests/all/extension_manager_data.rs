use std::fs;

use codex_plus_core::{
    extension_manager_data as public,
    gateway::{
        activity::{ActivitySnapshot, AgentActivity},
        key_pool::{KeyHealthView, PoolSnapshot, ProviderHealthView},
        quota::{ProviderQuota, QuotaWindow},
        speed::SpeedView,
        store::{GatewayGroup, GatewayProvider, GatewaySettings, GatewayStore, GroupMember},
    },
    settings::RelayProfile,
    skills::{SkillEntry, SkillsManager},
    user_scripts::{MANAGER_DATA_SOURCES, UserScriptManager},
};
use serde_json::{Value, json};

fn gateway() -> GatewaySettings {
    let relay = RelayProfile {
        id: "provider".into(),
        name: "Provider".into(),
        model: "model".into(),
        model_list: "model".into(),
        api_key: "SECRET_KEY".into(),
        base_url: "https://SECRET_HOST".into(),
        auth_contents: "SECRET_AUTH".into(),
        model_metadata: "SECRET_METADATA".into(),
        ..RelayProfile::default()
    };
    let provider: GatewayProvider = serde_json::from_value(json!({"relay": relay, "enabled": true, "proxy": "SECRET_PROXY", "unknown": "SECRET_EXTRA"})).unwrap();
    let mut group = GatewayGroup {
        id: "group".into(),
        name: "Group".into(),
        members: vec![GroupMember {
            model: "provider/model".into(),
            weight: 1,
        }],
        ..GatewayGroup::default()
    };
    group
        .extra
        .insert("futureSecret".into(), json!("SECRET_GROUP"));
    GatewaySettings {
        providers: vec![provider],
        groups: vec![group],
        ..GatewaySettings::default()
    }
}

fn keys(value: &Value, expected: &[&str]) {
    let mut actual = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    // 白名单比较完整的字段集合，不依赖 serde_json 是否保留插入顺序。
    actual.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

#[test]
fn provider_and_routing_views_only_return_explicit_public_fields() {
    let gateway = gateway();
    let providers = public::gateway_providers(&gateway);
    keys(&providers, &["providers"]);
    keys(
        &providers["providers"][0],
        &[
            "id",
            "name",
            "enabled",
            "protocol",
            "models",
            "modelWindows",
            "contextWindow",
            "keyRpm",
            "keyConcurrency",
            "price",
            "modelPrices",
        ],
    );
    assert_eq!(providers["providers"][0]["id"], "provider");
    let routing = public::gateway_routing(&gateway);
    keys(&routing, &["groups", "autoGroups"]);
    keys(
        &routing["groups"][0],
        &[
            "id",
            "name",
            "mode",
            "sticky",
            "members",
            "hideMembers",
            "pick",
        ],
    );
    assert_eq!(
        routing["groups"][0]["members"][0],
        json!({"model": "provider/model", "weight": 1})
    );
    assert!(!providers.to_string().contains("SECRET"));
    assert!(!routing.to_string().contains("SECRET"));
}

#[test]
fn health_snapshot_excludes_keys_and_unknown_providers() {
    let health = PoolSnapshot {
        keys: vec![KeyHealthView {
            relay_id: "provider".into(),
            key_id: "SECRET_KEY_ID".into(),
            state: "disabled".into(),
            remaining_ms: None,
            last_failure: Some("SECRET_ERROR".into()),
            last_failure_at_ms: None,
        }],
        providers: vec![
            ProviderHealthView {
                relay_id: "provider".into(),
                breaker_open: true,
                remaining_ms: Some(50),
                consecutive_errors: 3,
            },
            ProviderHealthView {
                relay_id: "SECRET_REMOVED".into(),
                breaker_open: false,
                remaining_ms: None,
                consecutive_errors: 0,
            },
        ],
    };
    let speeds = vec![SpeedView {
        relay_id: "provider".into(),
        model: "model".into(),
        ms: 50,
        samples: 2,
    }];
    let activity = ActivitySnapshot {
        agents: vec![AgentActivity {
            agent: "codex".into(),
            requests: 4,
            rerouted: 1,
            errors: 2,
            in_flight: vec!["provider".into(), "SECRET_REMOVED".into()],
            last: None,
        }],
    };
    let view = public::gateway_health(&gateway(), &health, &speeds, &activity);
    assert_eq!(view["scope"], "managerProcess");
    assert_eq!(view["providers"].as_array().unwrap().len(), 1);
    assert_eq!(view["agents"][0]["inFlight"], json!(["provider"]));
    keys(
        &view["providers"][0],
        &["relayId", "breakerOpen", "remainingMs", "consecutiveErrors"],
    );
    assert!(!view.to_string().contains("SECRET"));
    assert!(view.get("keys").is_none());
}

#[test]
fn quota_view_distinguishes_missing_cache_from_zero_usage() {
    let missing = public::gateway_quota(&gateway(), |_| None);
    assert_eq!(missing["cachedOnly"], true);
    assert_eq!(missing["providers"][0]["quota"], Value::Null);
    let view = public::gateway_quota(&gateway(), |id| {
        assert_eq!(id, "provider");
        Some(ProviderQuota {
            windows: vec![QuotaWindow {
                window_minutes: Some(300),
                used_percent: 0.0,
                resets_at_ms: Some(100),
                ..QuotaWindow::default()
            }],
            updated_at_ms: 10,
            ..ProviderQuota::default()
        })
    });
    assert_eq!(
        view["providers"][0]["quota"]["windows"][0]["usedPercent"],
        0.0
    );
    assert_eq!(view["providers"][0]["quota"]["updatedAtMs"], 10);
}

#[test]
fn skills_view_excludes_paths_descriptions_and_uninstalled_entries() {
    let entry = SkillEntry {
        id: "skill".into(),
        name: "Skill".into(),
        description: "SECRET_DESCRIPTION".into(),
        repo_key: "SECRET_REPO".into(),
        repo_path: "SECRET_PATH".into(),
        installed: true,
        enabled: true,
        bundled: false,
        content_hash: "hash".into(),
        remote_hash: "SECRET_REMOTE_HASH".into(),
        update_available: false,
        agents: vec!["codex".into()],
        managed: true,
    };
    let mut remote = entry.clone();
    remote.installed = false;
    let view = public::installed_skills(&[entry, remote]);
    assert_eq!(view["skills"].as_array().unwrap().len(), 1);
    keys(
        &view["skills"][0],
        &[
            "id",
            "name",
            "enabled",
            "bundled",
            "managed",
            "agents",
            "contentHash",
        ],
    );
    assert!(!view.to_string().contains("SECRET"));
}

#[test]
fn missing_stores_are_read_without_creating_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("absent");
    let gateway = GatewayStore::at(root.join("gateway.json")).load().unwrap();
    assert_eq!(
        public::gateway_providers(&gateway),
        json!({"providers": []})
    );
    let skills = SkillsManager::new(
        root.join("skills"),
        root.join("backups"),
        root.join("skills.json"),
        root.join("codex"),
    );
    assert_eq!(
        public::installed_skills(&skills.local_inventory()),
        json!({"skills": []})
    );
    assert!(!root.exists());
}

#[test]
fn manager_data_authorization_rechecks_current_manifest_and_enablement() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("user/panel");
    fs::create_dir_all(&root).unwrap();
    let manifest =
        json!({"id": "panel", "manager": {"page": "panel.html", "data": MANAGER_DATA_SOURCES}});
    fs::write(root.join("codexpp.json"), manifest.to_string()).unwrap();
    fs::write(root.join("panel.html"), "<p>panel</p>").unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        temp.path().join("user"),
        temp.path().join("scripts.json"),
    );
    for name in MANAGER_DATA_SOURCES {
        manager.authorize_manager_data("user:panel", name).unwrap();
    }
    assert!(
        manager
            .authorize_manager_data("user:panel", "settings")
            .is_err()
    );
    assert!(
        manager
            .authorize_manager_data("user:missing", "gatewayProviders")
            .is_err()
    );
    assert!(!temp.path().join("scripts.json").exists());
    manager.set_script_enabled("user:panel", false).unwrap();
    assert!(
        manager
            .authorize_manager_data("user:panel", "gatewayProviders")
            .is_err()
    );
    manager.set_script_enabled("user:panel", true).unwrap();
    manager.set_global_enabled(false).unwrap();
    assert!(
        manager
            .authorize_manager_data("user:panel", "gatewayProviders")
            .is_err()
    );
    manager.set_global_enabled(true).unwrap();
    let mut manifest = manifest;
    manifest["manager"]["data"] = json!(["gatewayQuota"]);
    fs::write(root.join("codexpp.json"), manifest.to_string()).unwrap();
    assert!(
        manager
            .authorize_manager_data("user:panel", "gatewayProviders")
            .is_err()
    );
    manager
        .authorize_manager_data("user:panel", "gatewayQuota")
        .unwrap();
    manifest["manager"]["data"] = json!(["notAllowed"]);
    fs::write(root.join("codexpp.json"), manifest.to_string()).unwrap();
    assert!(
        manager
            .authorize_manager_data("user:panel", "gatewayQuota")
            .is_err()
    );
}
