//! Codex 直连供应商在共享供应商里是「引用」不是复制：gateway.json 只存引用记录，
//! 地址、Key、模型这些每次读取时从 settings.json 现取；删掉直连供应商，共享里的这一项连同引用一起去掉。

use std::collections::BTreeMap;

use codex_plus_core::gateway::apply::{
    ProviderInput, add_provider, add_provider_key, create_provider, create_provider_owned,
    delete_provider, save_group, set_pool_member, set_provider_enabled, set_provider_extras,
    set_provider_limits, set_provider_metadata, set_provider_windows, update_provider, GroupInput,
};
use codex_plus_core::gateway::codex_link::{self, owner_of};
use codex_plus_core::gateway::model_sync::{apply_upstream_sync, set_provider_models};
use codex_plus_core::gateway::route;
use codex_plus_core::gateway::store::{
    AgentBinding, DefaultModelSelection, GatewayProvider, GatewayStore, GroupMember, ProviderPrice,
};
use codex_plus_core::settings::{RelayMode, RelayProfile, RelayProtocol, SettingsStore};
use codex_plus_core::tools::ToolId;

struct Fixture {
    _temp: tempfile::TempDir,
    gateway: GatewayStore,
    settings: SettingsStore,
}

/// gateway.json 与 settings.json 放在同一个临时目录：非默认位置的 gateway.json 读同目录的 settings.json。
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let fixture = Fixture {
        gateway: GatewayStore::at(temp.path().join("gateway.json")),
        settings: SettingsStore::new(temp.path().join("settings.json")),
        _temp: temp,
    };
    // 池里先放一个本地服务页加的供应商，gateway.json 就存在了。
    create_provider(&fixture.gateway, &pool_input("Kimi", "https://api.kimi.test/v1", "kimi-k3"), "preset:kimi", None)
        .unwrap();
    fixture
}

fn pool_input(name: &str, base_url: &str, model: &str) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        base_url: base_url.into(),
        protocol: RelayProtocol::ChatCompletions,
        keys: vec!["sk-pool".into()],
        model: model.into(),
        model_list: model.into(),
        ..ProviderInput::default()
    }
}

fn direct(id: &str, name: &str, key: &str, models: &[&str]) -> RelayProfile {
    RelayProfile {
        id: id.into(),
        name: name.into(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::ChatCompletions,
        base_url: format!("https://{id}.direct.test/v1"),
        api_key: key.into(),
        model: models[0].into(),
        model_list: models.join("\n"),
        ..RelayProfile::default()
    }
}

fn save_profiles(fixture: &Fixture, profiles: Vec<RelayProfile>) {
    let mut settings = fixture.settings.load().unwrap();
    settings.relay_profiles = profiles;
    fixture.settings.save(&settings).unwrap();
}

fn edit_profile(fixture: &Fixture, id: &str, edit: impl FnOnce(&mut RelayProfile)) {
    let mut settings = fixture.settings.load().unwrap();
    edit(settings.relay_profiles.iter_mut().find(|relay| relay.id == id).unwrap());
    fixture.settings.save(&settings).unwrap();
}

fn linked_id(store: &GatewayStore, relay_id: &str) -> String {
    store
        .load()
        .unwrap()
        .providers
        .iter()
        .find(|provider| provider.linked_codex_relay.as_deref() == Some(relay_id))
        .unwrap_or_else(|| panic!("{relay_id} 没有出现在共享供应商里"))
        .relay
        .id
        .clone()
}

fn gateway_text(fixture: &Fixture) -> String {
    std::fs::read_to_string(&fixture.gateway.path).unwrap()
}

fn enable_claude(store: &GatewayStore) {
    store
        .update(|gateway| {
            gateway.bindings.insert(
                ToolId::Claude.as_str().to_string(),
                AgentBinding { enabled: true, token: "t".into(), ..AgentBinding::default() },
            );
            Ok(())
        })
        .unwrap();
}

#[test]
fn eligible_direct_profiles_are_shared_by_reference_not_copied() {
    let fixture = fixture();
    save_profiles(&fixture, vec![direct("relay-a", "中转 A", "sk-direct-a", &["model-a", "model-b"])]);

    let gateway = fixture.gateway.load().unwrap();
    let id = linked_id(&fixture.gateway, "relay-a");
    assert_eq!(id, "中转-a", "新引用的 id 和旧版复制时一样按名字取");
    let provider = gateway.provider(&id).unwrap();
    assert_eq!(provider.source, "codex:relay-a");
    assert_eq!(owner_of(provider), Some("codex"));
    assert_eq!(provider.relay.base_url, "https://relay-a.direct.test/v1");
    assert_eq!(provider.relay.api_key, "sk-direct-a");
    assert_eq!(route::relay_models(&provider.relay), vec!["model-a", "model-b"]);

    // 读一次就写回了引用记录（id 从此固定），但连接资料不进 gateway.json。
    let text = gateway_text(&fixture);
    assert!(text.contains("\"linkedCodexRelay\": \"relay-a\""), "{text}");
    for secret in ["sk-direct-a", "relay-a.direct.test", "model-b"] {
        assert!(!text.contains(secret), "gateway.json 里不该有 {secret}：{text}");
    }
    // 再读、再写别的设置也不会把它们写进去。
    set_provider_enabled(&fixture.gateway, "kimi", false).unwrap();
    assert!(!gateway_text(&fixture).contains("sk-direct-a"));
}

#[test]
fn edits_to_the_direct_profile_reach_routing_and_model_lists_immediately() {
    let fixture = fixture();
    save_profiles(&fixture, vec![direct("relay-a", "Relay A", "sk-old", &["model-a"])]);
    enable_claude(&fixture.gateway);
    let id = linked_id(&fixture.gateway, "relay-a");

    let gateway = fixture.gateway.load().unwrap();
    let plan = route::resolve_plan(&gateway, &ToolId::Claude, &format!("{id}/model-a"), None).unwrap();
    assert_eq!(plan.candidates[0].relay.base_url, "https://relay-a.direct.test/v1");
    assert_eq!(plan.candidates[0].relay.api_key, "sk-old");

    // 在 Codex「连接与供应商」里换了地址、Key 和模型。
    edit_profile(&fixture, "relay-a", |relay| {
        // 编辑页改字段时连带重写 config / auth 快照；这里清掉快照，保存时按字段重新生成。
        relay.config_contents.clear();
        relay.auth_contents.clear();
        relay.base_url = "https://moved.direct.test/v1".into();
        relay.upstream_base_url = relay.base_url.clone();
        relay.api_key = "sk-new".into();
        relay.api_keys.clear();
        relay.active_api_key_id.clear();
        relay.model = "model-z".into();
        relay.model_list = "model-z".into();
        relay.model_windows = r#"{"model-z":"256K"}"#.into();
    });
    let gateway = fixture.gateway.load().unwrap();
    assert_eq!(linked_id(&fixture.gateway, "relay-a"), id, "改了直连供应商，id 不变");
    let plan = route::resolve_plan(&gateway, &ToolId::Claude, &format!("{id}/model-z"), None).unwrap();
    assert_eq!(plan.candidates[0].relay.base_url, "https://moved.direct.test/v1");
    assert_eq!(plan.candidates[0].relay.api_key, "sk-new");
    assert_eq!(plan.candidates[0].window, Some(256_000), "窗口也跟着直连供应商");
    let binding = gateway.binding(&ToolId::Claude).unwrap();
    let exposed = route::binding_exposed_models(&gateway, binding);
    assert!(exposed.iter().any(|model| model.ends_with("model-z")), "{exposed:?}");
    assert!(!exposed.iter().any(|model| model.ends_with("model-a")), "{exposed:?}");
    let pool = route::pool_models(&gateway, binding);
    assert!(pool.iter().any(|model| model.ends_with("model-z")), "{pool:?}");
    assert!(!gateway_text(&fixture).contains("sk-new"));
}

#[test]
fn deleting_the_direct_profile_removes_the_entry_and_its_references() {
    let fixture = fixture();
    save_profiles(
        &fixture,
        vec![
            direct("relay-a", "Relay A", "sk-a", &["model-a"]),
            direct("relay-b", "Relay B", "sk-b", &["model-b"]),
        ],
    );
    let id = linked_id(&fixture.gateway, "relay-a");
    save_group(
        &fixture.gateway,
        &GroupInput {
            name: "Fast".into(),
            members: vec![
                GroupMember { model: format!("{id}/model-a"), weight: 1 },
                GroupMember { model: "kimi/kimi-k3".into(), weight: 1 },
            ],
            ..GroupInput::default()
        },
    )
    .unwrap();
    fixture
        .gateway
        .update(|gateway| {
            let binding = gateway.bindings.entry("claude".into()).or_default();
            binding.model = format!("{id}/model-a");
            binding.extra_models = vec![format!("{id}/model-a"), "kimi/kimi-k3".into()];
            binding.default_model_selection = Some(DefaultModelSelection {
                provider_id: id.clone(),
                upstream_model: "model-a".into(),
                provider_source: "codex:relay-a".into(),
                provider_removed: false,
            });
            binding.pool_exclude = vec![id.clone()];
            Ok(())
        })
        .unwrap();

    // 在 Codex 里删掉 relay-a。
    let mut settings = fixture.settings.load().unwrap();
    settings.relay_profiles.retain(|relay| relay.id != "relay-a");
    fixture.settings.save(&settings).unwrap();

    let gateway = fixture.gateway.load().unwrap();
    assert!(gateway.provider(&id).is_none());
    assert!(gateway.providers.iter().any(|provider| provider.linked_codex_relay.as_deref() == Some("relay-b")));
    let group = gateway.groups.iter().find(|group| group.name == "Fast").unwrap();
    assert_eq!(group.members.iter().map(|member| member.model.as_str()).collect::<Vec<_>>(), vec!["kimi/kimi-k3"]);
    let binding = gateway.binding(&ToolId::Claude).unwrap();
    assert!(binding.model.is_empty());
    assert_eq!(binding.extra_models, vec!["kimi/kimi-k3".to_string()]);
    assert!(binding.default_model_selection.is_none());
    assert!(binding.pool_exclude.is_empty());
    // 清理写回了文件。
    assert!(!gateway_text(&fixture).contains("relay-a"), "{}", gateway_text(&fixture));
}

#[test]
fn existing_copies_turn_into_references_in_place() {
    let fixture = fixture();
    // 旧版「加入共享供应商」复制的：直连供应商还没写进 settings.json 时它就是个普通复制品。
    let copy = direct("relay-a", "Relay A", "sk-stale-copy", &["model-a"]);
    let (_, id) = add_provider(&fixture.gateway, copy, "codex:relay-a", None).unwrap();
    set_provider_extras(
        &fixture.gateway,
        &id,
        Some(ProviderPrice { currency: "USD".into(), input: 1.0, cached_input: 0.1, output: 2.0 }),
        None,
        "",
    )
    .unwrap();
    save_group(
        &fixture.gateway,
        &GroupInput {
            name: "Fast".into(),
            members: vec![GroupMember { model: format!("{id}/model-a"), weight: 1 }],
            ..GroupInput::default()
        },
    )
    .unwrap();
    assert!(fixture.gateway.load().unwrap().provider(&id).unwrap().linked_codex_relay.is_none());

    save_profiles(&fixture, vec![direct("relay-a", "Relay A", "sk-live", &["model-a"])]);
    let gateway = fixture.gateway.load().unwrap();
    let provider = gateway.provider(&id).unwrap();
    assert_eq!(provider.linked_codex_relay.as_deref(), Some("relay-a"), "原地转成引用，id 不变");
    assert_eq!(provider.relay.api_key, "sk-live");
    assert_eq!(provider.price.as_ref().unwrap().input, 1.0, "价格这些共享供应商自己的设置留着");
    assert_eq!(
        gateway.providers.iter().filter(|provider| provider.source == "codex:relay-a").count(),
        1,
        "不多出一条"
    );
    assert_eq!(gateway.groups[0].members[0].model, format!("{id}/model-a"));
    let text = gateway_text(&fixture);
    assert!(!text.contains("sk-stale-copy"), "复制品的连接资料从 gateway.json 里去掉：{text}");
}

#[test]
fn profiles_that_cannot_be_shared_are_not_linked() {
    let fixture = fixture();
    let aggregate = RelayProfile {
        id: "agg".into(),
        name: "聚合".into(),
        relay_mode: RelayMode::Aggregate,
        ..RelayProfile::default()
    };
    let official = RelayProfile {
        id: "official".into(),
        name: "ChatGPT".into(),
        relay_mode: RelayMode::Official,
        protocol: RelayProtocol::Responses,
        ..RelayProfile::default()
    };
    let gateway_managed = RelayProfile {
        id: codex_plus_core::gateway::apply::CODEX_GATEWAY_RELAY_ID.into(),
        ..direct("managed", "本地服务", "cpx-gw-codex-x", &["group/fast"])
    };
    let mut self_loop = direct("loop", "环回", "sk-loop", &["m"]);
    self_loop.base_url = "http://127.0.0.1:57400/gw/codex/v1".into();
    let mut official_mix = direct("mix", "官方混合", "sk-mix", &["gpt-6"]);
    official_mix.relay_mode = RelayMode::Official;
    official_mix.official_mix_api_key = true;
    official_mix.protocol = RelayProtocol::Responses;
    save_profiles(
        &fixture,
        vec![aggregate.clone(), official.clone(), gateway_managed.clone(), self_loop.clone(), official_mix.clone()],
    );
    for relay in [&aggregate, &official, &gateway_managed, &self_loop] {
        assert!(!codex_link::linkable(relay), "{} 不该能共享", relay.id);
    }
    assert!(codex_link::rejection(&official).unwrap().contains("API Key"));

    let gateway = fixture.gateway.load().unwrap();
    let linked: Vec<&str> = gateway
        .providers
        .iter()
        .filter_map(|provider| provider.linked_codex_relay.as_deref())
        .collect();
    assert_eq!(linked, vec!["mix"], "只有带 API Key 的官方混合供应商能共享（旧规则）");
}

#[test]
fn linked_connection_fields_are_read_only_in_the_pool() {
    let fixture = fixture();
    save_profiles(&fixture, vec![direct("relay-a", "Relay A", "sk-a", &["model-a"])]);
    let id = linked_id(&fixture.gateway, "relay-a");
    let store = &fixture.gateway;
    let rejected: Vec<(&str, anyhow::Result<()>)> = vec![
        ("编辑", update_provider(store, &id, &pool_input("X", "https://x.test/v1", "x")).map(drop)),
        ("加 Key", add_provider_key(store, &id, "sk-extra").map(drop)),
        ("改窗口", set_provider_windows(store, &id, BTreeMap::new(), Some(1_000)).map(drop)),
        ("改模型资料", set_provider_metadata(store, &id, "{}").map(drop)),
        ("改模型列表", set_provider_models(store, &id, &["y".to_string()], None).map(drop)),
        ("同步模型", apply_upstream_sync(store, &id, &[], &["y".to_string()]).map(drop)),
    ];
    for (what, result) in rejected {
        let error = format!("{:#}", result.expect_err(what));
        assert!(error.contains("连接与供应商"), "{what}：{error}");
    }
    let error = format!("{:#}", delete_provider(store, &id).unwrap_err());
    assert!(error.contains("不能在这里删除"), "{error}");

    // 共享供应商自己的设置照常能改，写进引用记录。
    set_provider_extras(
        store,
        &id,
        Some(ProviderPrice { currency: "USD".into(), input: 3.0, cached_input: 0.0, output: 9.0 }),
        None,
        "http://127.0.0.1:7890",
    )
    .unwrap();
    set_provider_limits(store, &id, Some(5), None).unwrap();
    set_pool_member(store, &[ToolId::Claude], &id, false).unwrap();
    set_provider_enabled(store, &id, false).unwrap();
    let gateway = store.load().unwrap();
    let provider = gateway.provider(&id).unwrap();
    assert_eq!(provider.price.as_ref().unwrap().output, 9.0);
    assert_eq!(provider.proxy, "http://127.0.0.1:7890");
    assert_eq!(provider.key_rpm, Some(5));
    assert!(!provider.enabled);
    assert_eq!(provider.relay.api_key, "sk-a", "连接资料没被改动");
    assert!(gateway.binding(&ToolId::Claude).unwrap().pool_exclude.contains(&id));
}

#[test]
fn new_links_respect_agents_that_do_not_auto_join() {
    let fixture = fixture();
    fixture
        .gateway
        .update(|gateway| {
            gateway.bindings.insert(
                "claude".into(),
                AgentBinding { auto_join_providers: false, ..AgentBinding::default() },
            );
            Ok(())
        })
        .unwrap();
    save_profiles(&fixture, vec![direct("relay-a", "Relay A", "sk-a", &["model-a"])]);
    let id = linked_id(&fixture.gateway, "relay-a");
    let gateway = fixture.gateway.load().unwrap();
    assert_eq!(gateway.binding(&ToolId::Claude).unwrap().pool_exclude, vec![id]);
    // 再读多少次也只记一次。
    let again = fixture.gateway.load().unwrap();
    assert_eq!(again.binding(&ToolId::Claude).unwrap().pool_exclude.len(), 1);
}

#[test]
fn owners_come_from_links_the_claude_page_and_sources() {
    let provider = |source: &str, owner: &str| {
        let mut provider = GatewayProvider::new(RelayProfile::default(), source);
        provider.owner = owner.into();
        provider
    };
    assert_eq!(owner_of(&GatewayProvider::linked("a", "relay-a")), Some("codex"));
    assert_eq!(owner_of(&provider("codex:relay-x", "")), Some("codex"), "旧版复制品也算 Codex 的");
    for source in ["claude:p", "ccswitch:p", "claudemom:p", "magpie:@gateway"] {
        assert_eq!(owner_of(&provider(source, "")), Some("claude"), "{source}");
    }
    assert_eq!(owner_of(&provider("preset:deepseek", "claude")), Some("claude"));
    assert_eq!(owner_of(&provider("preset:deepseek", "")), None, "本地服务页加的两边都列");
    assert_eq!(owner_of(&provider("login:chatgpt", "")), None);

    // Claude 页加的记下归属；别的页面传什么都不记。
    let fixture = fixture();
    let (_, claude) = create_provider_owned(
        &fixture.gateway,
        &pool_input("Anthropic", "https://api.anthropic.test", "claude-x"),
        "custom",
        Some("claude"),
        None,
    )
    .unwrap();
    let (_, other) = create_provider_owned(
        &fixture.gateway,
        &pool_input("Other", "https://other.test/v1", "o"),
        "custom",
        Some("codex"),
        None,
    )
    .unwrap();
    let gateway = fixture.gateway.load().unwrap();
    assert_eq!(gateway.provider(&claude).unwrap().owner, "claude");
    assert_eq!(owner_of(gateway.provider(&other).unwrap()), None);
    assert!(gateway_text(&fixture).contains("\"owner\": \"claude\""));
}

#[test]
fn unreadable_settings_leave_links_alone() {
    let fixture = fixture();
    save_profiles(&fixture, vec![direct("relay-a", "Relay A", "sk-a", &["model-a"])]);
    let id = linked_id(&fixture.gateway, "relay-a");
    // settings.json 写坏了（不是删掉）：这次不同步，引用和模型组都不动。
    std::fs::write(fixture.gateway.path.with_file_name("settings.json"), "{ broken").unwrap();
    let gateway = fixture.gateway.load().unwrap();
    assert_eq!(gateway.provider(&id).unwrap().linked_codex_relay.as_deref(), Some("relay-a"));
    assert!(gateway_text(&fixture).contains("linkedCodexRelay"));
}

#[test]
fn temporarily_unshareable_profiles_keep_their_link_and_references() {
    let fixture = fixture();
    save_profiles(&fixture, vec![direct("relay-a", "Relay A", "sk-a", &["model-a"])]);
    let id = linked_id(&fixture.gateway, "relay-a");
    save_group(
        &fixture.gateway,
        &GroupInput {
            name: "Fast".into(),
            members: vec![GroupMember { model: format!("{id}/model-a"), weight: 1 }],
            ..GroupInput::default()
        },
    )
    .unwrap();
    fixture
        .gateway
        .update(|gateway| {
            gateway.bindings.entry("claude".into()).or_default().model = format!("{id}/model-a");
            Ok(())
        })
        .unwrap();

    // 在 Codex 里改成聚合：暂时不能共享，但引用、模型组、默认模型都留着，只是没有模型。
    edit_profile(&fixture, "relay-a", |relay| relay.relay_mode = RelayMode::Aggregate);
    let gateway = fixture.gateway.load().unwrap();
    let provider = gateway.provider(&id).expect("引用留着");
    assert_eq!(provider.linked_codex_relay.as_deref(), Some("relay-a"));
    assert!(provider.relay.base_url.is_empty() && provider.relay.model_list.is_empty(), "不填连接资料");
    assert_eq!(gateway.groups[0].members[0].model, format!("{id}/model-a"));
    assert_eq!(gateway.binding(&ToolId::Claude).unwrap().model, format!("{id}/model-a"));

    // 改回直连就恢复，id 不变。
    edit_profile(&fixture, "relay-a", |relay| relay.relay_mode = RelayMode::PureApi);
    let gateway = fixture.gateway.load().unwrap();
    assert_eq!(gateway.provider(&id).unwrap().relay.api_key, "sk-a");
    assert_eq!(linked_id(&fixture.gateway, "relay-a"), id);

    // 已填上连接资料的不能删；空引用可以删。
    assert!(delete_provider(&fixture.gateway, &id).is_err());
    edit_profile(&fixture, "relay-a", |relay| relay.relay_mode = RelayMode::Aggregate);
    delete_provider(&fixture.gateway, &id).unwrap();
    assert!(fixture.gateway.load().unwrap().provider(&id).is_none());
}

#[test]
fn missing_settings_file_leaves_links_alone() {
    let fixture = fixture();
    save_profiles(&fixture, vec![direct("relay-a", "Relay A", "sk-a", &["model-a"])]);
    let id = linked_id(&fixture.gateway, "relay-a");
    std::fs::remove_file(fixture.gateway.path.with_file_name("settings.json")).unwrap();
    let gateway = fixture.gateway.load().unwrap();
    assert_eq!(gateway.provider(&id).unwrap().linked_codex_relay.as_deref(), Some("relay-a"));
}
