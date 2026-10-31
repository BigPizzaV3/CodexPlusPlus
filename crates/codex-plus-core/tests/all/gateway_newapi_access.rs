//! 供应商池里 new-api 的访问令牌（#2793）：按供应商保存，给界面的只有打码后的令牌；
//! 留空沿用、可单独清除，用户 ID 只能是数字；供应商地址换到别的站点时令牌一起丢掉。

use codex_plus_core::gateway::apply::{
    ProviderInput, create_provider, provider_newapi_access, set_provider_newapi_access,
    update_provider,
};
use codex_plus_core::gateway::store::{GatewayProvider, GatewayStore};
use codex_plus_core::settings::RelayProtocol;

const TOKEN: &str = "nap_access-0123456789abcdefghij-TTTT";

fn input(name: &str, base_url: &str) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        base_url: base_url.into(),
        protocol: RelayProtocol::ChatCompletions,
        keys: vec!["sk-relay-0123456789abcdefghij-KKKK".into()],
        model: "m1".into(),
        model_list: "m1".into(),
        ..ProviderInput::default()
    }
}

fn setup(name: &str) -> (tempfile::TempDir, GatewayStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(temp.path().join("gateway.json"));
    let (_, id) = create_provider(
        &store,
        &input(name, "https://newapi.example.test/v1"),
        "custom",
        None,
    )
    .unwrap();
    (temp, store, id)
}

fn provider(store: &GatewayStore, id: &str) -> GatewayProvider {
    store
        .load()
        .unwrap()
        .providers
        .into_iter()
        .find(|provider| provider.relay.id == id)
        .unwrap()
}

#[test]
fn access_token_is_saved_per_provider_and_shown_masked() {
    let (_temp, store, id) = setup("NewApi Access 2793");
    assert!(provider_newapi_access(&provider(&store, &id)).is_none());

    set_provider_newapi_access(&store, &id, Some(&format!("  {TOKEN}\n")), " 42 ", false).unwrap();

    let saved = provider(&store, &id);
    assert_eq!(saved.newapi_access.as_ref().unwrap().token, TOKEN);
    let view = provider_newapi_access(&saved).unwrap();
    assert_eq!(view.user_id, "42");
    assert!(!view.masked.contains(TOKEN));
    let json = serde_json::to_string(&view).unwrap();
    assert!(!json.contains(TOKEN), "给界面的数据不能有令牌原文：{json}");
    // 打印调试信息也不带令牌原文。
    assert!(!format!("{saved:?}").contains(TOKEN));
}

#[test]
fn empty_token_keeps_saved_one_and_clear_removes_it() {
    let (_temp, store, id) = setup("NewApi Keep 2793");
    // 还没存过令牌时不能只填用户 ID。
    assert!(set_provider_newapi_access(&store, &id, None, "1", false).is_err());

    set_provider_newapi_access(&store, &id, Some(TOKEN), "1", false).unwrap();
    set_provider_newapi_access(&store, &id, Some("  "), "7", false).unwrap();
    let access = provider(&store, &id).newapi_access.unwrap();
    assert_eq!(access.token, TOKEN);
    assert_eq!(access.user_id, "7");

    // 用户 ID 只能是数字；令牌里不能夹空白。
    assert!(set_provider_newapi_access(&store, &id, None, "abc", false).is_err());
    assert!(set_provider_newapi_access(&store, &id, Some("a b"), "", false).is_err());

    set_provider_newapi_access(&store, &id, None, "", true).unwrap();
    assert!(provider(&store, &id).newapi_access.is_none());
    assert!(
        !std::fs::read_to_string(&store.path)
            .unwrap()
            .contains(TOKEN)
    );
}

#[test]
fn editing_provider_keeps_token_on_same_site_and_drops_it_on_another() {
    let (_temp, store, id) = setup("NewApi Edit 2793");
    set_provider_newapi_access(&store, &id, Some(TOKEN), "1", false).unwrap();

    // 同一站点只改路径、名字：令牌保留。
    update_provider(
        &store,
        &id,
        &ProviderInput {
            keys: Vec::new(),
            ..input("NewApi Edit 2793 renamed", "https://newapi.example.test/")
        },
    )
    .unwrap();
    assert!(provider(&store, &id).newapi_access.is_some());

    // 换到别的站点：令牌不能跟过去。
    update_provider(
        &store,
        &id,
        &ProviderInput {
            keys: Vec::new(),
            ..input("NewApi Edit 2793 renamed", "https://other.example.test/v1")
        },
    )
    .unwrap();
    assert!(provider(&store, &id).newapi_access.is_none());
}
