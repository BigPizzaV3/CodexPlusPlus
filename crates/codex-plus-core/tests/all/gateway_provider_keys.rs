//! 供应商池里单独管理 API Key（#2782）：列出打码后的 Key，按标识添加、替换、删除，
//! 完整 Key 不出现在给界面的数据里；删改后 Key 池里对应的冷却、会话固定跟着清掉。
//!
//! Key 池状态是进程内全局的：每个用例用自己的供应商名，id 不和别的测试撞。

use std::time::{Duration, Instant};

use codex_plus_core::gateway::apply::{
    ProviderInput, add_provider_key, create_provider, delete_provider_key, mask_api_key,
    provider_keys, replace_provider_key, update_provider,
};
use codex_plus_core::gateway::key_pool::{self, FailureClass};
use codex_plus_core::gateway::store::{GatewayStore, KeyRotationMode};
use codex_plus_core::settings::{RelayMode, RelayProfile, RelayProtocol};

const KEY_A: &str = "sk-alpha-0123456789abcdefghij-AAAA";
const KEY_B: &str = "sk-bravo-0123456789abcdefghij-BBBB";
const KEY_C: &str = "sk-charlie-0123456789abcdefgh-CCCC";
const KEY_NEW: &str = "sk-novel-9876543210zyxwvutsrq-NNNN";

fn input(name: &str, keys: &[&str]) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        base_url: "https://api.keys.test/v1".into(),
        protocol: RelayProtocol::ChatCompletions,
        keys: keys.iter().map(|key| key.to_string()).collect(),
        model: "m1".into(),
        model_list: "m1".into(),
        ..ProviderInput::default()
    }
}

fn setup(name: &str, keys: &[&str]) -> (tempfile::TempDir, GatewayStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(temp.path().join("gateway.json"));
    let (_, id) = create_provider(&store, &input(name, keys), "custom", None).unwrap();
    (temp, store, id)
}

fn relay(store: &GatewayStore, id: &str) -> RelayProfile {
    store
        .load()
        .unwrap()
        .providers
        .into_iter()
        .find(|provider| provider.relay.id == id)
        .unwrap()
        .relay
}

fn key_ids(store: &GatewayStore, id: &str) -> Vec<String> {
    provider_keys(&relay(store, id))
        .into_iter()
        .map(|key| key.id)
        .collect()
}

fn stored_text(store: &GatewayStore) -> String {
    std::fs::read_to_string(&store.path).unwrap()
}

#[test]
fn masks_keep_only_head_and_tail() {
    // 长 Key：前 6 后 4。
    assert_eq!(mask_api_key(KEY_A), "sk-alp…AAAA");
    assert_eq!(mask_api_key(&format!("  {KEY_A}\n")), "sk-alp…AAAA");
    // 短 Key 按长度少露，露出的不到三分之一。
    assert_eq!(mask_api_key("sk-1234567890"), "sk…0");
    assert_eq!(mask_api_key("abcdefg"), "a…");
    assert_eq!(mask_api_key("abcd"), "••••");
    assert_eq!(mask_api_key(""), "••••");
    for key in [KEY_A, "sk-1234567890", "abcdefghijklmnop"] {
        let shown = mask_api_key(key).chars().filter(|ch| *ch != '…').count();
        assert!(shown * 3 <= key.chars().count(), "{key} 露得太多");
    }
}

#[test]
fn saved_keys_are_listed_masked_with_stable_ids() {
    let (_temp, store, id) = setup("Keys List 2782", &[KEY_A, KEY_B]);
    let views = provider_keys(&relay(&store, &id));
    assert_eq!(
        views
            .iter()
            .map(|key| (key.id.as_str(), key.masked.as_str()))
            .collect::<Vec<_>>(),
        vec![("key-1", "sk-alp…AAAA"), ("key-2", "sk-bra…BBBB")]
    );
    // 给界面的 JSON 里没有完整 Key，也没有 Key 中间的内容。
    let json = serde_json::to_string(&views).unwrap();
    for secret in [KEY_A, KEY_B, "0123456789"] {
        assert!(!json.contains(secret), "{json}");
    }

    // 单 Key 供应商也列出来，标识是 default。
    let (_temp, single, single_id) = setup("Keys Single 2782", &[KEY_C]);
    assert_eq!(key_ids(&single, &single_id), vec!["default"]);

    // 订阅账号、不校验 Key 的本机服务不列 Key。
    let account = RelayProfile {
        id: "copilot-2782".into(),
        relay_mode: RelayMode::PureApi,
        api_key: "token".into(),
        auth_contents: r#"{"codex_plus_account":"copilot","OPENAI_API_KEY":"token"}"#.into(),
        ..RelayProfile::default()
    };
    assert!(provider_keys(&account).is_empty());
    let no_auth = RelayProfile {
        id: "local-2782".into(),
        relay_mode: RelayMode::PureApi,
        no_auth: true,
        ..RelayProfile::default()
    };
    assert!(provider_keys(&no_auth).is_empty());
}

#[test]
fn keys_are_added_replaced_and_deleted_by_id() {
    let (_temp, store, id) = setup("Keys Edit 2782", &[KEY_A, KEY_B]);
    add_provider_key(&store, &id, &format!(" {KEY_C} ")).unwrap();
    assert_eq!(key_ids(&store, &id), vec!["key-1", "key-2", "key-3"]);
    let error = add_provider_key(&store, &id, KEY_A).unwrap_err();
    assert!(format!("{error:#}").contains("已经在列表里"), "{error:#}");
    let error = add_provider_key(&store, &id, "sk-a sk-b").unwrap_err();
    assert!(format!("{error:#}").contains("不能有空格"), "{error:#}");
    assert!(add_provider_key(&store, &id, "  ").is_err());

    // 替换：标识不变，只换值；和别的 Key 重复时拒绝。
    replace_provider_key(&store, &id, "key-2", KEY_NEW).unwrap();
    let saved = relay(&store, &id);
    let values = saved
        .api_keys
        .iter()
        .map(|key| (key.id.as_str(), key.api_key.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![("key-1", KEY_A), ("key-2", KEY_NEW), ("key-3", KEY_C)]
    );
    assert!(replace_provider_key(&store, &id, "key-3", KEY_A).is_err());
    assert!(replace_provider_key(&store, &id, "key-9", "sk-whatever-123").is_err());
    assert!(
        !stored_text(&store).contains(KEY_B),
        "换掉的 Key 不留在文件里"
    );

    // 删掉当前选中的 Key：改选剩下的第一个，发请求用的 Key 跟上。
    assert_eq!(saved.active_api_key_id, "key-1");
    delete_provider_key(&store, &id, "key-1").unwrap();
    let saved = relay(&store, &id);
    assert_eq!(key_ids(&store, &id), vec!["key-2", "key-3"]);
    assert_eq!(saved.active_api_key_id, "key-2");
    assert_eq!(saved.api_key, KEY_NEW);
    assert!(
        !stored_text(&store).contains(KEY_A),
        "删掉的 Key 不留在文件里"
    );
    assert!(delete_provider_key(&store, &id, "key-1").is_err());

    // 再加一个不会和现有标识重复。
    add_provider_key(&store, &id, KEY_B).unwrap();
    assert_eq!(key_ids(&store, &id), vec!["key-2", "key-3", "key-4"]);
}

#[test]
fn the_last_key_can_be_replaced_but_not_deleted() {
    let (_temp, store, id) = setup("Keys Last 2782", &[KEY_A]);
    let error = delete_provider_key(&store, &id, "default").unwrap_err();
    assert!(
        format!("{error:#}").contains("至少要保留一个 Key"),
        "{error:#}"
    );
    replace_provider_key(&store, &id, "default", KEY_B).unwrap();
    assert_eq!(relay(&store, &id).api_key, KEY_B);
    assert!(!stored_text(&store).contains(KEY_A));

    let (_temp, store, id) = setup("Keys Last Two 2782", &[KEY_A, KEY_B]);
    delete_provider_key(&store, &id, "key-1").unwrap();
    assert!(delete_provider_key(&store, &id, "key-2").is_err());
    let saved = relay(&store, &id);
    assert_eq!(saved.api_key, KEY_B);
    assert_eq!(key_pool::expand_keys(&saved).len(), 1);
}

#[test]
fn deleting_or_replacing_a_key_clears_its_pool_state() {
    let (_temp, store, id) = setup("Keys Pool 2782", &[KEY_A, KEY_B, KEY_C]);
    let before = relay(&store, &id);
    let candidates = key_pool::expand_keys(&before);
    let key_of = |key_id: &str| {
        candidates
            .iter()
            .find(|candidate| candidate.key.key_id == key_id)
            .unwrap()
            .key
            .clone()
    };
    let now = Instant::now();
    key_pool::record_failure(
        &key_of("key-2"),
        &FailureClass::RateLimited {
            retry_after: Some(Duration::from_secs(600)),
        },
        now,
    );
    key_pool::record_success(&key_of("key-3"), Some("conversation-2782"));
    let pinned = key_pool::order_keys(
        &before,
        KeyRotationMode::Sticky,
        Some("conversation-2782"),
        now,
    );
    assert_eq!(pinned.0[0].key.key_id, "key-3");
    let cooling = |relay_id: &str, key_id: &str| {
        key_pool::snapshot(Instant::now())
            .keys
            .iter()
            .any(|key| key.relay_id == relay_id && key.key_id == key_id && key.state == "cooling")
    };
    assert!(cooling(&id, "key-2"));

    // 换掉 key-2：冷却跟着清掉。删掉会话固定的 key-3：会话回退到按策略选。
    replace_provider_key(&store, &id, "key-2", KEY_NEW).unwrap();
    assert!(!cooling(&id, "key-2"));
    delete_provider_key(&store, &id, "key-3").unwrap();
    let after = relay(&store, &id);
    let (available, _) = key_pool::order_keys(
        &after,
        KeyRotationMode::Sticky,
        Some("conversation-2782"),
        Instant::now(),
    );
    let order = available
        .iter()
        .map(|candidate| candidate.key.key_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(order, vec!["key-1", "key-2"]);
    key_pool::reset(&id);
}

#[test]
fn editing_a_provider_without_new_keys_keeps_key_ids() {
    let (_temp, store, id) = setup("Keys Keep 2782", &[KEY_A, KEY_B, KEY_C]);
    delete_provider_key(&store, &id, "key-1").unwrap();
    let mut edit = input("Keys Keep 2782 renamed", &[]);
    edit.model_list = "m1\nm2".into();
    update_provider(&store, &id, &edit).unwrap();
    let saved = relay(&store, &id);
    assert_eq!(saved.name, "Keys Keep 2782 renamed");
    assert_eq!(key_ids(&store, &id), vec!["key-2", "key-3"]);
    assert_eq!(saved.api_key, KEY_B);
}
