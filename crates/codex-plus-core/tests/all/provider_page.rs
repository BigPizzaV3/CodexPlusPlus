//! Codex 页面「供应商」页的后端：列表打码、提交时还原 Key、按需切换并重写 live 文件。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use codex_plus_core::gateway::apply::CODEX_GATEWAY_RELAY_ID;
use codex_plus_core::provider_page::{
    ProviderCommitRequest, ProviderPageHost, ProviderSwitchResult, api_key_preview,
    codex_gateway_mode, commit_providers, gateway_command_allowed, provider_list_value,
    redact_gateway_result, route_commit, route_gateway,
};
use codex_plus_core::relay_switch::switch_relay_profile_in_home_resolving_conflict;
use codex_plus_core::settings::{BackendSettings, RelayApiKey, RelayMode, RelayProfile, SettingsStore};
use serde_json::{Value, json};

const KEY_A: &str = "sk-a-secret-0000aaaa";
const KEY_B1: &str = "sk-b-secret-1111bbbb";
const KEY_B2: &str = "sk-b-backup-2222cccc";

fn pure_profile(id: &str, base_url: &str, keys: &[(&str, &str)]) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: id.to_uppercase(),
        relay_mode: RelayMode::PureApi,
        config_contents: format!(
            "model_provider = \"custom\"\n\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nrequires_openai_auth = true\nbase_url = \"{base_url}\"\n"
        ),
        auth_contents: format!(r#"{{"OPENAI_API_KEY":"{}"}}"#, keys[0].1),
        api_keys: keys
            .iter()
            .map(|(key_id, key)| RelayApiKey {
                id: key_id.to_string(),
                name: key_id.to_uppercase(),
                api_key: key.to_string(),
            })
            .collect(),
        active_api_key_id: keys[0].0.to_string(),
        ..RelayProfile::default()
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    store: SettingsStore,
    home: PathBuf,
    switches: Arc<AtomicUsize>,
    saves: Arc<AtomicUsize>,
}

impl Fixture {
    fn new(extra: Vec<RelayProfile>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        std::fs::create_dir(&home).unwrap();
        let store = SettingsStore::new(temp.path().join("settings.json"));
        let mut relay_profiles = vec![
            pure_profile("a", "https://a.example/v1", &[("ka", KEY_A)]),
            pure_profile("b", "https://b.example/v1", &[("kb1", KEY_B1), ("kb2", KEY_B2)]),
        ];
        relay_profiles.extend(extra);
        store
            .save(&BackendSettings { active_relay_id: "a".to_string(), relay_profiles, ..BackendSettings::default() })
            .unwrap();
        // 先按真实流程把 a 写进 live 文件，后续切换的回填才有基线。
        let settings = store.load().unwrap();
        switch_relay_profile_in_home_resolving_conflict(&store, &home, settings, "", Default::default()).unwrap();
        Self {
            _temp: temp,
            store,
            home,
            switches: Arc::new(AtomicUsize::new(0)),
            saves: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// 测试宿主：保存走带 revision 的落盘，切换走 core 的真实切换流程（与管理器命令同一个入口）。
    fn host(&self) -> ProviderPageHost {
        let save_store = self.store.clone();
        let switch_store = self.store.clone();
        let home = self.home.clone();
        let saves = self.saves.clone();
        let switches = self.switches.clone();
        ProviderPageHost {
            save: Arc::new(move |mut settings: BackendSettings, revision: Option<String>| {
                saves.fetch_add(1, Ordering::SeqCst);
                if let Some(revision) = revision {
                    settings.settings_revision = revision;
                }
                save_store.save_versioned(&settings).map(|_| "设置已保存。".to_string()).map_err(|error| format!("{error:#}"))
            }),
            switch: Arc::new(move |settings: BackendSettings, previous: String, resolution| {
                switches.fetch_add(1, Ordering::SeqCst);
                match switch_relay_profile_in_home_resolving_conflict(&switch_store, &home, settings, &previous, resolution) {
                    Ok(_) => ProviderSwitchResult { ok: true, message: "供应商已切换。".to_string(), live_conflict: None },
                    Err(error) => ProviderSwitchResult { ok: false, message: format!("{error:#}"), live_conflict: None },
                }
            }),
            gateway: Arc::new(|command: &str, args: Value| json!({ "status": "ok", "command": command, "args": args })),
        }
    }

    fn list(&self) -> Value {
        provider_list_value(&self.store.load().unwrap()).unwrap()
    }

    fn commit(&self, request: Value) -> anyhow::Result<codex_plus_core::provider_page::ProviderCommitOutcome> {
        let request: ProviderCommitRequest = serde_json::from_value(request).unwrap();
        commit_providers(&self.store, &self.host(), &request)
    }

    fn stored(&self, id: &str) -> RelayProfile {
        self.store.load().unwrap().relay_profiles.into_iter().find(|profile| profile.id == id).unwrap()
    }

    fn live(&self, name: &str) -> String {
        std::fs::read_to_string(self.home.join(name)).unwrap_or_default()
    }
}

fn profiles_of(list: &Value) -> Vec<Value> {
    list["settings"]["relayProfiles"].as_array().unwrap().clone()
}

fn profile_mut<'a>(profiles: &'a mut [Value], id: &str) -> &'a mut Value {
    profiles.iter_mut().find(|profile| profile["id"] == id).unwrap()
}

fn key_values(profile: &RelayProfile) -> Vec<(String, String)> {
    profile.api_keys.iter().map(|key| (key.id.clone(), key.api_key.clone())).collect()
}

fn live_mentions(fixture: &Fixture, needle: &str) -> bool {
    fixture.live("config.toml").contains(needle) || fixture.live("auth.json").contains(needle)
}

#[test]
fn list_masks_named_api_keys() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    let b = profiles_of(&list).into_iter().find(|profile| profile["id"] == "b").unwrap();
    let keys = serde_json::to_string(&b["apiKeys"]).unwrap();
    assert!(!keys.contains("secret") && !keys.contains("backup"), "{keys}");
    assert_eq!(b["apiKeys"][0]["preview"], api_key_preview(KEY_B1));
    assert_eq!(b["apiKeys"][0]["stored"], true);
    assert_eq!(b["apiKeys"][0]["apiKey"], "");
    assert_eq!(api_key_preview(KEY_B1), "sk-…bbbb");
    assert_eq!(api_key_preview("short"), "••••");
    assert_eq!(list["revision"], fixture.store.revision().unwrap());
    assert_eq!(list["gatewayRelayId"], CODEX_GATEWAY_RELAY_ID);
}

#[test]
fn editing_inactive_profile_restores_masked_keys_and_only_saves() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    let mut profiles = profiles_of(&list);
    profile_mut(&mut profiles, "b")["name"] = json!("B 改名");

    let outcome = fixture
        .commit(json!({ "revision": list["revision"], "relayProfiles": profiles }))
        .unwrap();

    assert!(!outcome.switched);
    assert_eq!(fixture.switches.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.saves.load(Ordering::SeqCst), 1);
    let b = fixture.stored("b");
    assert_eq!(b.name, "B 改名");
    assert_eq!(
        key_values(&b),
        vec![("kb1".to_string(), KEY_B1.to_string()), ("kb2".to_string(), KEY_B2.to_string())]
    );
    assert_eq!(fixture.store.load().unwrap().active_relay_id, "a");
}

#[test]
fn editing_active_profile_rewrites_live_files() {
    let fixture = Fixture::new(Vec::new());
    assert!(fixture.live("config.toml").contains("https://a.example/v1"));
    let list = fixture.list();
    let mut profiles = profiles_of(&list);
    let a = profile_mut(&mut profiles, "a");
    let config = a["configContents"].as_str().unwrap().replace("https://a.example/v1", "https://a2.example/v1");
    a["configContents"] = json!(config);

    fixture.commit(json!({ "revision": list["revision"], "relayProfiles": profiles })).unwrap();

    assert_eq!(fixture.switches.load(Ordering::SeqCst), 1);
    assert!(fixture.live("config.toml").contains("https://a2.example/v1"), "{}", fixture.live("config.toml"));
    assert!(live_mentions(&fixture, KEY_A), "当前供应商的 Key 必须还原成真值写进 live 文件");
}

#[test]
fn unchanged_active_profile_is_not_reapplied() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    fixture.commit(json!({ "revision": list["revision"], "relayProfiles": profiles_of(&list) })).unwrap();
    assert_eq!(fixture.switches.load(Ordering::SeqCst), 0, "原样提交不应重写 live 文件");
}

#[test]
fn activate_switches_and_uses_selected_restored_key() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    let mut profiles = profiles_of(&list);
    // 页面里只改「当前 Key」：两把都是打码的，服务端还原后按选中的那把写文件。
    profile_mut(&mut profiles, "b")["activeApiKeyId"] = json!("kb2");

    let outcome = fixture
        .commit(json!({ "revision": list["revision"], "relayProfiles": profiles, "activate": "b" }))
        .unwrap();

    assert!(outcome.switched, "{}", outcome.message);
    assert_eq!(fixture.store.load().unwrap().active_relay_id, "b");
    assert!(fixture.live("config.toml").contains("https://b.example/v1"));
    assert!(live_mentions(&fixture, KEY_B2), "auth={} config={}", fixture.live("auth.json"), fixture.live("config.toml"));
    assert!(!live_mentions(&fixture, KEY_B1));
}

#[test]
fn stale_revision_is_rejected() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    let error = fixture
        .commit(json!({ "revision": "stale", "relayProfiles": profiles_of(&list) }))
        .unwrap_err();
    assert!(error.to_string().contains("settings_revision_conflict"), "{error}");
    assert_eq!(fixture.saves.load(Ordering::SeqCst) + fixture.switches.load(Ordering::SeqCst), 0);
}

#[test]
fn active_profile_cannot_be_removed() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    let profiles: Vec<Value> = profiles_of(&list).into_iter().filter(|profile| profile["id"] != "a").collect();
    let error = fixture.commit(json!({ "revision": list["revision"], "relayProfiles": profiles })).unwrap_err();
    assert!(error.to_string().contains("当前使用的供应商不能删除"), "{error}");
    assert_eq!(fixture.store.load().unwrap().relay_profiles.len(), 2);
}

#[test]
fn inactive_profile_can_be_removed() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    let profiles: Vec<Value> = profiles_of(&list).into_iter().filter(|profile| profile["id"] != "b").collect();
    fixture.commit(json!({ "revision": list["revision"], "relayProfiles": profiles })).unwrap();
    let ids: Vec<String> = fixture.store.load().unwrap().relay_profiles.into_iter().map(|profile| profile.id).collect();
    assert_eq!(ids, vec!["a".to_string()]);
}

#[test]
fn duplicate_takes_keys_from_key_source() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    let mut profiles = profiles_of(&list);
    let mut copy = profile_mut(&mut profiles, "b").clone();
    copy["id"] = json!("b-copy");
    copy["name"] = json!("B 副本");
    profiles.push(copy);

    fixture
        .commit(json!({ "revision": list["revision"], "relayProfiles": profiles, "keySources": { "b-copy": "b" } }))
        .unwrap();

    assert_eq!(key_values(&fixture.stored("b-copy")), key_values(&fixture.stored("b")));
}

#[test]
fn new_plain_key_is_kept_and_unknown_masked_key_is_dropped() {
    let fixture = Fixture::new(Vec::new());
    let list = fixture.list();
    let mut profiles = profiles_of(&list);
    let b = profile_mut(&mut profiles, "b");
    b["apiKeys"] = json!([
        { "id": "kb1", "name": "KB1", "apiKey": "", "stored": true },
        { "id": "ghost", "name": "已被删", "apiKey": "", "stored": true },
        { "id": "kb3", "name": "新 Key", "apiKey": "sk-b-new-3333dddd" },
    ]);

    fixture.commit(json!({ "revision": list["revision"], "relayProfiles": profiles })).unwrap();

    assert_eq!(
        key_values(&fixture.stored("b")),
        vec![("kb1".to_string(), KEY_B1.to_string()), ("kb3".to_string(), "sk-b-new-3333dddd".to_string())]
    );
}

#[test]
fn gateway_entry_is_kept_from_storage() {
    let gateway = RelayProfile {
        name: "Codex++ 网关".to_string(),
        ..pure_profile(CODEX_GATEWAY_RELAY_ID, "http://127.0.0.1:57321/gw/codex/v1", &[("gw", "sk-gateway-local-9999")])
    };
    let fixture = Fixture::new(vec![gateway.clone()]);
    let list = fixture.list();
    let gateway_row = profiles_of(&list).into_iter().find(|profile| profile["id"] == CODEX_GATEWAY_RELAY_ID).unwrap();
    assert_eq!(gateway_row["gatewayManaged"], true);

    // 页面既想改它也想删它：两种都不生效。
    let mut profiles: Vec<Value> = profiles_of(&list);
    profile_mut(&mut profiles, CODEX_GATEWAY_RELAY_ID)["name"] = json!("被页面改了");
    fixture.commit(json!({ "revision": list["revision"], "relayProfiles": profiles })).unwrap();
    assert_eq!(fixture.stored(CODEX_GATEWAY_RELAY_ID).name, gateway.name);

    let list = fixture.list();
    let profiles: Vec<Value> = profiles_of(&list).into_iter().filter(|profile| profile["id"] != CODEX_GATEWAY_RELAY_ID).collect();
    fixture.commit(json!({ "revision": list["revision"], "relayProfiles": profiles })).unwrap();
    assert_eq!(key_values(&fixture.stored(CODEX_GATEWAY_RELAY_ID)), key_values(&gateway));
}

#[test]
fn commit_route_requires_registered_host() {
    let error = route_commit(&json!({ "relayProfiles": [] })).unwrap_err();
    assert!(error.to_string().contains("管理器未运行"), "{error}");
}

#[test]
fn gateway_commands_are_whitelisted_and_codex_only() {
    assert!(gateway_command_allowed("load_gateway", &json!({})).is_ok());
    assert!(gateway_command_allowed("update_gateway_provider", &json!({ "id": "x" })).is_ok());
    // 白名单外：池以外的写能力（模型组、订阅登录、价格）不给页面。
    for command in ["save_gateway_group", "login_gateway_chatgpt", "save_gateway_provider_extras", "load_settings"] {
        assert!(gateway_command_allowed(command, &json!({})).is_err(), "{command}");
    }
    // 按 Agent 生效的命令只能作用在 Codex 上。
    for command in ["save_gateway_binding", "preview_gateway_disconnect", "disable_gateway_binding", "disable_gateway_binding_full_switch"] {
        assert!(gateway_command_allowed(command, &json!({ "agent": "codex" })).is_ok(), "{command}");
        assert!(gateway_command_allowed(command, &json!({ "agent": "claude" })).is_err(), "{command}");
        assert!(gateway_command_allowed(command, &json!({})).is_err(), "{command}");
    }
    // 拒绝发生在找宿主之前：未注册宿主时白名单外命令也报「不支持」，不会碰管理器。
    let error = route_gateway(&json!({ "command": "save_gateway_group", "args": {} })).unwrap_err();
    assert!(error.to_string().contains("不支持"), "{error}");
}

#[test]
fn gateway_results_drop_local_gateway_tokens() {
    let redacted = redact_gateway_result(json!({
        "status": "ok",
        "gateway": { "bindings": { "codex": { "enabled": true, "token": "local-secret" }, "claude": { "token": "x" } }, "providers": [] },
        "providers": [{ "id": "p", "keys": [{ "id": "k", "masked": "sk-…abcd" }] }],
    }));
    let text = redacted.to_string();
    assert!(!text.contains("local-secret") && !text.contains("\"token\""), "{text}");
    assert_eq!(redacted["gateway"]["bindings"]["codex"]["enabled"], true);
    assert_eq!(redacted["providers"][0]["keys"][0]["masked"], "sk-…abcd");
}

#[test]
fn gateway_mode_requires_gateway_profile_master_switch_and_binding() {
    let mut settings = BackendSettings { active_relay_id: CODEX_GATEWAY_RELAY_ID.to_string(), relay_profiles_enabled: true, ..BackendSettings::default() };
    assert!(codex_gateway_mode(&settings, true));
    assert!(!codex_gateway_mode(&settings, false), "绑定关了不算网关模式");
    settings.relay_profiles_enabled = false;
    assert!(!codex_gateway_mode(&settings, true), "总开关关了不算");
    settings.relay_profiles_enabled = true;
    settings.active_relay_id = "a".to_string();
    assert!(!codex_gateway_mode(&settings, true), "当前不是网关条目不算");
}
